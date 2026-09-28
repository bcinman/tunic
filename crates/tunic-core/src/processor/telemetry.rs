//! Processor telemetry values and the private latest-value transport.
//!
//! Subscriptions create demand for analysis. The processor publishes complete
//! frames without blocking, and clients poll the newest frame when convenient.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub const SPECTRUM_POINT_COUNT: usize = 256;
pub const SPECTRUM_MIN_FREQUENCY_HZ: f32 = 20.0;
pub const SPECTRUM_MAX_FREQUENCY_HZ: f32 = 20_000.0;

const ACTIVE_SUBSCRIPTION_MASK: u64 = u32::MAX as u64;
const GENERATION_INCREMENT: u64 = 1_u64 << 32;

/// Returns the logarithmically spaced center frequency for a spectrum point.
///
/// Indices beyond the end of the spectrum are clamped to the final point.
#[must_use]
pub fn spectrum_frequency_hz(index: usize) -> f32 {
    let ratio = index.min(SPECTRUM_POINT_COUNT - 1) as f32 / (SPECTRUM_POINT_COUNT - 1) as f32;
    SPECTRUM_MIN_FREQUENCY_HZ * (SPECTRUM_MAX_FREQUENCY_HZ / SPECTRUM_MIN_FREQUENCY_HZ).powf(ratio)
}

/// One post-processor measurement snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TelemetryFrame {
    pub levels: StereoLevels,
    pub spectrum: Spectrum,
}

/// Peak and RMS levels for both output channels, in linear amplitude.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StereoLevels {
    pub left: ChannelLevels,
    pub right: ChannelLevels,
}

/// Peak and RMS levels for one output channel, in linear amplitude.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChannelLevels {
    pub peak: f32,
    pub rms: f32,
}

/// Logarithmically spaced post-processor magnitudes in linear amplitude.
///
/// Point frequencies span 20 Hz through 20 kHz and are obtained with
/// [`spectrum_frequency_hz`]. Points above the stream's Nyquist frequency are
/// zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spectrum {
    pub points: [f32; SPECTRUM_POINT_COUNT],
}

impl Default for Spectrum {
    fn default() -> Self {
        Self {
            points: [0.0; SPECTRUM_POINT_COUNT],
        }
    }
}

/// A read-only latest-value view of measurements produced by a processor.
///
/// Clones share one subscription. Calling `Controller::subscribe_telemetry`
/// again creates independent demand. The first frame is available only after
/// the processor has observed enough new audio for a complete analysis.
#[derive(Clone)]
pub struct Telemetry {
    subscription: Arc<TelemetrySubscription>,
}

impl Telemetry {
    /// Returns the latest complete frame without blocking.
    #[must_use]
    pub fn latest(&self) -> Option<TelemetryFrame> {
        let shared = &self.subscription.shared;
        let before = shared.sequence.load(Ordering::Acquire);
        if !before.is_multiple_of(2) {
            return None;
        }
        let frame = shared.load();
        let published_generation =
            TelemetryGeneration(shared.published_generation.load(Ordering::SeqCst));
        let after = shared.sequence.load(Ordering::Acquire);
        (before == after && published_generation == self.subscription.generation).then_some(frame)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TelemetryGeneration(u32);

#[derive(Clone)]
pub(crate) struct TelemetrySource {
    shared: Arc<SharedTelemetry>,
}

impl TelemetrySource {
    pub(crate) fn subscribe(&self) -> Telemetry {
        let previous = self
            .shared
            .demand
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                let subscriptions = state & ACTIVE_SUBSCRIPTION_MASK;
                if subscriptions == ACTIVE_SUBSCRIPTION_MASK {
                    None
                } else if subscriptions == 0 {
                    Some(state.wrapping_add(GENERATION_INCREMENT + 1))
                } else {
                    Some(state + 1)
                }
            })
            .expect("telemetry subscription count overflow");
        let generation = if previous & ACTIVE_SUBSCRIPTION_MASK == 0 {
            generation(previous).0.wrapping_add(1)
        } else {
            generation(previous).0
        };
        Telemetry {
            subscription: Arc::new(TelemetrySubscription {
                shared: Arc::clone(&self.shared),
                generation: TelemetryGeneration(generation),
            }),
        }
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

pub(crate) struct TelemetryPublisher {
    shared: Arc<SharedTelemetry>,
}

impl TelemetryPublisher {
    #[must_use]
    pub(crate) fn active_generation(&self) -> Option<TelemetryGeneration> {
        let demand = self.shared.demand.load(Ordering::Acquire);
        (demand & ACTIVE_SUBSCRIPTION_MASK != 0).then(|| generation(demand))
    }

    pub(crate) fn publish(&self, generation: TelemetryGeneration, frame: TelemetryFrame) {
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
    spectrum: [AtomicU32; SPECTRUM_POINT_COUNT],
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
        for (target, value) in self.spectrum.iter().zip(frame.spectrum.points) {
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
                points: std::array::from_fn(|index| {
                    f32::from_bits(self.spectrum[index].load(Ordering::SeqCst))
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ChannelLevels, Spectrum, StereoLevels, TelemetryFrame, channel};

    fn frame() -> TelemetryFrame {
        TelemetryFrame {
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
                points: std::array::from_fn(|index| index as f32 / 100.0),
            },
        }
    }

    #[test]
    fn subscription_returns_the_latest_complete_frame() {
        let (publisher, source) = channel();
        let telemetry = source.subscribe();
        let frame = frame();

        assert_eq!(telemetry.latest(), None);
        publisher.publish(telemetry.subscription.generation, frame);
        assert_eq!(telemetry.latest(), Some(frame));
    }

    #[test]
    fn demand_tracks_independent_subscriptions_not_clones() {
        let (publisher, source) = channel();
        assert_eq!(publisher.active_generation(), None);

        let telemetry = source.subscribe();
        let clone = telemetry.clone();
        let independent = source.subscribe();
        let generation = telemetry.subscription.generation;
        assert_eq!(publisher.active_generation(), Some(generation));

        drop(telemetry);
        drop(clone);
        assert_eq!(publisher.active_generation(), Some(generation));
        drop(independent);
        assert_eq!(publisher.active_generation(), None);
    }

    #[test]
    fn resubscription_waits_for_a_fresh_frame() {
        let (publisher, source) = channel();
        let previous = source.subscribe();
        let previous_generation = previous.subscription.generation;
        publisher.publish(previous_generation, frame());
        assert!(previous.latest().is_some());

        drop(previous);
        let current = source.subscribe();
        let current_generation = current.subscription.generation;
        assert_ne!(current_generation, previous_generation);
        assert_eq!(current.latest(), None);

        publisher.publish(previous_generation, frame());
        assert_eq!(current.latest(), None);
        publisher.publish(current_generation, TelemetryFrame::default());
        assert_eq!(current.latest(), Some(TelemetryFrame::default()));
    }
}
