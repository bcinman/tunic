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
const FRAME_HISTORY_CAPACITY: usize = 16;

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
    /// Monotonically increasing publication number for this complete snapshot.
    pub sequence: u64,
    pub levels: StereoLevels,
    pub spectrum: Spectrum,
}

/// Raw peak and RMS measurements for both output channels, in linear amplitude.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StereoLevels {
    pub left: ChannelLevels,
    pub right: ChannelLevels,
}

/// Raw peak and RMS measurements for one output channel, in linear amplitude.
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

/// A read-only view of recent measurements produced by a processor.
///
/// Clones share one subscription. Calling `Controller::subscribe_telemetry`
/// again creates independent demand. The first frame is available only after
/// the processor has observed enough new audio for a complete analysis. Clones
/// also share the cursor used by [`Self::for_each_unseen`].
#[derive(Clone)]
pub struct Telemetry {
    subscription: Arc<TelemetrySubscription>,
}

impl Telemetry {
    /// Returns the latest complete frame without blocking.
    #[must_use]
    pub fn latest(&self) -> Option<TelemetryFrame> {
        let sequence = self
            .subscription
            .shared
            .published_sequence
            .load(Ordering::Acquire);
        self.subscription
            .shared
            .load(sequence, self.subscription.generation)
    }

    /// Visits every complete frame published since the previous call.
    ///
    /// If the reader falls more than 16 frames behind, only the retained tail is
    /// visited. This method does not block or allocate.
    pub fn for_each_unseen(&self, mut visit: impl FnMut(TelemetryFrame)) {
        let latest = self
            .subscription
            .shared
            .published_sequence
            .load(Ordering::Acquire);
        let previous = self
            .subscription
            .last_seen_sequence
            .swap(latest, Ordering::AcqRel);
        if latest <= previous {
            return;
        }
        let oldest_retained = latest.saturating_sub(FRAME_HISTORY_CAPACITY as u64 - 1);
        for sequence in previous.saturating_add(1).max(oldest_retained)..=latest {
            if let Some(frame) = self
                .subscription
                .shared
                .load(sequence, self.subscription.generation)
            {
                visit(frame);
            }
        }
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
        let initial_sequence = self.shared.published_sequence.load(Ordering::Acquire);
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
                last_seen_sequence: AtomicU64::new(initial_sequence),
            }),
        }
    }
}

struct TelemetrySubscription {
    shared: Arc<SharedTelemetry>,
    generation: TelemetryGeneration,
    last_seen_sequence: AtomicU64,
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
        let sequence = self
            .shared
            .published_sequence
            .load(Ordering::Relaxed)
            .checked_add(1)
            .expect("telemetry sequence overflow");
        self.shared.store(sequence, generation, frame);
        self.shared
            .published_sequence
            .store(sequence, Ordering::Release);
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
    published_sequence: AtomicU64,
    frames: [SharedTelemetryFrame; FRAME_HISTORY_CAPACITY],
}

struct SharedTelemetryFrame {
    sequence: AtomicU64,
    generation: AtomicU32,
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
            published_sequence: AtomicU64::new(0),
            frames: std::array::from_fn(|_| SharedTelemetryFrame::default()),
        }
    }
}

impl Default for SharedTelemetryFrame {
    fn default() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            generation: AtomicU32::new(0),
            left_peak: AtomicU32::new(0),
            left_rms: AtomicU32::new(0),
            right_peak: AtomicU32::new(0),
            right_rms: AtomicU32::new(0),
            spectrum: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
}

impl SharedTelemetry {
    fn store(&self, sequence: u64, generation: TelemetryGeneration, frame: TelemetryFrame) {
        let target = &self.frames[sequence as usize % FRAME_HISTORY_CAPACITY];
        target.sequence.store(0, Ordering::Release);
        target.generation.store(generation.0, Ordering::SeqCst);
        target
            .left_peak
            .store(frame.levels.left.peak.to_bits(), Ordering::SeqCst);
        target
            .left_rms
            .store(frame.levels.left.rms.to_bits(), Ordering::SeqCst);
        target
            .right_peak
            .store(frame.levels.right.peak.to_bits(), Ordering::SeqCst);
        target
            .right_rms
            .store(frame.levels.right.rms.to_bits(), Ordering::SeqCst);
        for (point, value) in target.spectrum.iter().zip(frame.spectrum.points) {
            point.store(value.to_bits(), Ordering::SeqCst);
        }
        target.sequence.store(sequence, Ordering::Release);
    }

    fn load(&self, sequence: u64, generation: TelemetryGeneration) -> Option<TelemetryFrame> {
        if sequence == 0 {
            return None;
        }
        let source = &self.frames[sequence as usize % FRAME_HISTORY_CAPACITY];
        let before = source.sequence.load(Ordering::Acquire);
        if before != sequence
            || TelemetryGeneration(source.generation.load(Ordering::SeqCst)) != generation
        {
            return None;
        }
        let frame = TelemetryFrame {
            sequence,
            levels: StereoLevels {
                left: ChannelLevels {
                    peak: f32::from_bits(source.left_peak.load(Ordering::SeqCst)),
                    rms: f32::from_bits(source.left_rms.load(Ordering::SeqCst)),
                },
                right: ChannelLevels {
                    peak: f32::from_bits(source.right_peak.load(Ordering::SeqCst)),
                    rms: f32::from_bits(source.right_rms.load(Ordering::SeqCst)),
                },
            },
            spectrum: Spectrum {
                points: std::array::from_fn(|index| {
                    f32::from_bits(source.spectrum[index].load(Ordering::SeqCst))
                }),
            },
        };
        (source.sequence.load(Ordering::Acquire) == sequence).then_some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChannelLevels, FRAME_HISTORY_CAPACITY, Spectrum, StereoLevels, TelemetryFrame, channel,
    };

    fn frame() -> TelemetryFrame {
        TelemetryFrame {
            sequence: 0,
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
        assert_eq!(
            telemetry.latest(),
            Some(TelemetryFrame {
                sequence: 1,
                ..frame
            })
        );
    }

    #[test]
    fn subscription_visits_every_unseen_frame_in_publication_order() {
        let (publisher, source) = channel();
        let telemetry = source.subscribe();
        let generation = telemetry.subscription.generation;
        for peak in [0.2, 0.9, 0.1] {
            let mut frame = frame();
            frame.levels.left.peak = peak;
            publisher.publish(generation, frame);
        }

        let mut seen = Vec::new();
        telemetry.for_each_unseen(|frame| {
            seen.push((frame.sequence, frame.levels.left.peak));
        });

        assert_eq!(seen, [(1, 0.2), (2, 0.9), (3, 0.1)]);
        telemetry.for_each_unseen(|frame| seen.push((frame.sequence, frame.levels.left.peak)));
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn slow_subscription_receives_the_retained_tail() {
        let (publisher, source) = channel();
        let telemetry = source.subscribe();
        let generation = telemetry.subscription.generation;
        for peak in 1..=FRAME_HISTORY_CAPACITY + 2 {
            let mut frame = frame();
            frame.levels.left.peak = peak as f32;
            publisher.publish(generation, frame);
        }

        let mut seen = Vec::new();
        telemetry.for_each_unseen(|frame| seen.push(frame));

        assert_eq!(seen.len(), FRAME_HISTORY_CAPACITY);
        assert_eq!(seen.first().unwrap().sequence, 3);
        assert_eq!(seen.first().unwrap().levels.left.peak, 3.0);
        assert_eq!(seen.last().unwrap().sequence, 18);
        assert_eq!(seen.last().unwrap().levels.left.peak, 18.0);
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
        assert_eq!(
            current.latest(),
            Some(TelemetryFrame {
                sequence: 2,
                ..TelemetryFrame::default()
            })
        );
    }
}
