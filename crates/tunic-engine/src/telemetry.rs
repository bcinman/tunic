use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

const ACTIVE_READER_MASK: u64 = u32::MAX as u64;
const GENERATION_INCREMENT: u64 = 1_u64 << 32;

pub const SPECTRUM_BAND_COUNT: usize = 28;
pub const SPECTRUM_FREQUENCIES_HZ: [f32; SPECTRUM_BAND_COUNT] = [
    31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0, 630.0,
    800.0, 1_000.0, 1_250.0, 1_600.0, 2_000.0, 2_500.0, 3_150.0, 4_000.0, 5_000.0, 6_300.0,
    8_000.0, 10_000.0, 12_500.0, 16_000.0,
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChannelLevels {
    pub peak: f32,
    pub rms: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StereoLevels {
    pub left: ChannelLevels,
    pub right: ChannelLevels,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spectrum {
    pub bands: [f32; SPECTRUM_BAND_COUNT],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TelemetryFrame {
    pub levels: StereoLevels,
    pub spectrum: Spectrum,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelemetryGeneration(u32);

pub(super) struct TelemetrySource {
    shared: Arc<SharedTelemetry>,
}

impl TelemetrySource {
    pub(super) fn subscribe(&self) -> TelemetryReader {
        let previous = self
            .shared
            .demand
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                let readers = state & ACTIVE_READER_MASK;
                if readers == ACTIVE_READER_MASK {
                    None
                } else if readers == 0 {
                    Some(state.wrapping_add(GENERATION_INCREMENT + 1))
                } else {
                    Some(state + 1)
                }
            })
            .expect("telemetry reader count overflow");
        let generation = if previous & ACTIVE_READER_MASK == 0 {
            generation(previous).0.wrapping_add(1)
        } else {
            generation(previous).0
        };
        TelemetryReader {
            subscription: Arc::new(TelemetrySubscription {
                shared: Arc::clone(&self.shared),
                generation: TelemetryGeneration(generation),
            }),
        }
    }
}

#[derive(Clone)]
pub struct TelemetryReader {
    subscription: Arc<TelemetrySubscription>,
}

impl TelemetryReader {
    #[must_use]
    pub fn try_latest(&self) -> Option<TelemetryFrame> {
        let shared = &self.subscription.shared;
        let before = shared.sequence.load(Ordering::Acquire);
        if !before.is_multiple_of(2) {
            return None;
        }
        let levels = shared.load();
        let published_generation =
            TelemetryGeneration(shared.published_generation.load(Ordering::SeqCst));
        let after = shared.sequence.load(Ordering::Acquire);
        (before == after && published_generation == self.subscription.generation).then_some(levels)
    }
}

struct TelemetrySubscription {
    shared: Arc<SharedTelemetry>,
    generation: TelemetryGeneration,
}

impl Drop for TelemetrySubscription {
    fn drop(&mut self) {
        self.shared.demand.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
pub struct TelemetryPublisher {
    shared: Arc<SharedTelemetry>,
}

impl TelemetryPublisher {
    #[must_use]
    pub fn active_generation(&self) -> Option<TelemetryGeneration> {
        let demand = self.shared.demand.load(Ordering::Acquire);
        (demand & ACTIVE_READER_MASK != 0).then(|| generation(demand))
    }

    pub fn publish(&self, generation: TelemetryGeneration, frame: TelemetryFrame) {
        if self.active_generation() != Some(generation) {
            return;
        }
        let sequence = self.shared.sequence.load(Ordering::Acquire);
        if !sequence.is_multiple_of(2)
            || self
                .shared
                .sequence
                .compare_exchange(
                    sequence,
                    sequence.wrapping_add(1),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
        {
            return;
        }
        if self.active_generation() != Some(generation) {
            self.shared
                .sequence
                .store(sequence.wrapping_add(2), Ordering::Release);
            return;
        }
        self.shared.store(frame);
        self.shared
            .published_generation
            .store(generation.0, Ordering::SeqCst);
        self.shared
            .sequence
            .store(sequence.wrapping_add(2), Ordering::Release);
    }
}

fn generation(demand: u64) -> TelemetryGeneration {
    TelemetryGeneration((demand >> 32) as u32)
}

pub(crate) fn channel() -> (TelemetryPublisher, TelemetrySource) {
    let shared = Arc::new(SharedTelemetry::default());
    (
        TelemetryPublisher {
            shared: Arc::clone(&shared),
        },
        TelemetrySource { shared },
    )
}

struct SharedTelemetry {
    demand: AtomicU64,
    sequence: AtomicU64,
    published_generation: AtomicU32,
    left_peak: AtomicU32,
    left_rms: AtomicU32,
    right_peak: AtomicU32,
    right_rms: AtomicU32,
    spectrum: [AtomicU32; SPECTRUM_BAND_COUNT],
}

impl Default for SharedTelemetry {
    fn default() -> Self {
        Self {
            demand: AtomicU64::new(0),
            sequence: AtomicU64::new(0),
            published_generation: AtomicU32::new(0),
            left_peak: AtomicU32::new(0),
            left_rms: AtomicU32::new(0),
            right_peak: AtomicU32::new(0),
            right_rms: AtomicU32::new(0),
            spectrum: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
}

impl SharedTelemetry {
    fn store(&self, frame: TelemetryFrame) {
        self.left_peak
            .store(frame.levels.left.peak.to_bits(), Ordering::SeqCst);
        self.left_rms
            .store(frame.levels.left.rms.to_bits(), Ordering::SeqCst);
        self.right_peak
            .store(frame.levels.right.peak.to_bits(), Ordering::SeqCst);
        self.right_rms
            .store(frame.levels.right.rms.to_bits(), Ordering::SeqCst);
        for (target, value) in self.spectrum.iter().zip(frame.spectrum.bands) {
            target.store(value.to_bits(), Ordering::SeqCst);
        }
    }

    fn load(&self) -> TelemetryFrame {
        TelemetryFrame {
            levels: StereoLevels {
                left: ChannelLevels {
                    peak: f32::from_bits(self.left_peak.load(Ordering::SeqCst)),
                    rms: f32::from_bits(self.left_rms.load(Ordering::SeqCst)),
                },
                right: ChannelLevels {
                    peak: f32::from_bits(self.right_peak.load(Ordering::SeqCst)),
                    rms: f32::from_bits(self.right_rms.load(Ordering::SeqCst)),
                },
            },
            spectrum: Spectrum {
                bands: std::array::from_fn(|index| {
                    f32::from_bits(self.spectrum[index].load(Ordering::SeqCst))
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{ChannelLevels, Spectrum, StereoLevels, TelemetryFrame, channel};

    #[test]
    fn reader_returns_the_latest_complete_publication() {
        let (publisher, source) = channel();
        let reader = source.subscribe();
        let frame = TelemetryFrame {
            levels: StereoLevels {
                left: ChannelLevels {
                    peak: 0.75,
                    rms: 0.5,
                },
                right: ChannelLevels {
                    peak: 0.25,
                    rms: 0.125,
                },
            },
            spectrum: Spectrum {
                bands: std::array::from_fn(|index| index as f32 / 100.0),
            },
        };

        assert_eq!(reader.try_latest(), None);
        publisher.publish(reader.subscription.generation, frame);
        assert_eq!(reader.try_latest(), Some(frame));
    }

    #[test]
    fn reader_does_not_wait_for_an_in_progress_publication() {
        let (publisher, source) = channel();
        let reader = source.subscribe();
        publisher.shared.sequence.store(1, Ordering::Release);

        assert_eq!(reader.try_latest(), None);
    }

    #[test]
    fn concurrent_publisher_does_not_write_into_an_owned_publication() {
        let (publisher, source) = channel();
        let reader = source.subscribe();
        let initial = TelemetryFrame {
            levels: StereoLevels {
                left: ChannelLevels {
                    peak: 0.75,
                    rms: 0.5,
                },
                right: ChannelLevels {
                    peak: 0.25,
                    rms: 0.125,
                },
            },
            spectrum: Spectrum::default(),
        };
        let generation = reader.subscription.generation;
        publisher.publish(generation, initial);
        let sequence = publisher.shared.sequence.load(Ordering::Acquire);
        publisher
            .shared
            .sequence
            .compare_exchange(sequence, sequence + 1, Ordering::AcqRel, Ordering::Acquire)
            .unwrap();

        let concurrent_publisher = publisher.clone();
        concurrent_publisher.publish(generation, TelemetryFrame::default());
        publisher
            .shared
            .sequence
            .store(sequence + 2, Ordering::Release);

        assert_eq!(reader.try_latest(), Some(initial));
    }

    #[test]
    fn telemetry_is_enabled_only_while_a_reader_exists() {
        let (publisher, source) = channel();
        assert_eq!(publisher.active_generation(), None);

        let reader = source.subscribe();
        let reader_clone = reader.clone();
        let independent_reader = source.subscribe();
        let generation = reader.subscription.generation;
        assert_eq!(publisher.active_generation(), Some(generation));

        drop(reader);
        assert_eq!(publisher.active_generation(), Some(generation));
        drop(reader_clone);
        assert_eq!(publisher.active_generation(), Some(generation));
        drop(independent_reader);
        assert_eq!(publisher.active_generation(), None);

        let sequence = publisher.shared.sequence.load(Ordering::Acquire);
        publisher.publish(generation, TelemetryFrame::default());
        assert_eq!(publisher.shared.sequence.load(Ordering::Acquire), sequence);
        let next_reader = source.subscribe();
        assert_ne!(next_reader.subscription.generation, generation);
        assert_eq!(next_reader.try_latest(), None);
    }

    #[test]
    fn rapid_resubscription_rejects_the_previous_generation() {
        let (publisher, source) = channel();
        let previous_reader = source.subscribe();
        let previous_generation = previous_reader.subscription.generation;
        let previous = TelemetryFrame {
            levels: StereoLevels {
                left: ChannelLevels {
                    peak: 0.75,
                    rms: 0.5,
                },
                right: ChannelLevels::default(),
            },
            spectrum: Spectrum::default(),
        };
        publisher.publish(previous_generation, previous);
        assert_eq!(previous_reader.try_latest(), Some(previous));

        drop(previous_reader);
        let current_reader = source.subscribe();
        let current_generation = current_reader.subscription.generation;

        assert_ne!(current_generation, previous_generation);
        assert_eq!(current_reader.try_latest(), None);
        publisher.publish(previous_generation, previous);
        assert_eq!(current_reader.try_latest(), None);

        let current = TelemetryFrame::default();
        publisher.publish(current_generation, current);
        assert_eq!(current_reader.try_latest(), Some(current));
    }

    #[test]
    fn reader_rejects_an_old_publication_completed_after_resubscription() {
        let (publisher, source) = channel();
        let previous_reader = source.subscribe();
        let previous_generation = previous_reader.subscription.generation;
        let sequence = publisher.shared.sequence.load(Ordering::Acquire);
        publisher
            .shared
            .sequence
            .compare_exchange(sequence, sequence + 1, Ordering::AcqRel, Ordering::Acquire)
            .unwrap();

        drop(previous_reader);
        let current_reader = source.subscribe();
        publisher.shared.store(TelemetryFrame::default());
        publisher
            .shared
            .published_generation
            .store(previous_generation.0, Ordering::SeqCst);
        publisher
            .shared
            .sequence
            .store(sequence + 2, Ordering::Release);

        assert_eq!(current_reader.try_latest(), None);
    }
}
