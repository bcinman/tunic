use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

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

#[derive(Clone)]
pub struct TelemetryReader {
    shared: Arc<SharedTelemetry>,
}

impl TelemetryReader {
    #[must_use]
    pub fn try_latest(&self) -> Option<TelemetryFrame> {
        let before = self.shared.sequence.load(Ordering::Acquire);
        if !before.is_multiple_of(2) {
            return None;
        }
        let levels = self.shared.load();
        let after = self.shared.sequence.load(Ordering::Acquire);
        (before == after).then_some(levels)
    }
}

#[derive(Clone)]
pub struct TelemetryPublisher {
    shared: Arc<SharedTelemetry>,
}

impl TelemetryPublisher {
    pub fn publish(&self, frame: TelemetryFrame) {
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
        self.shared.store(frame);
        self.shared
            .sequence
            .store(sequence.wrapping_add(2), Ordering::Release);
    }
}

pub(crate) fn channel() -> (TelemetryPublisher, TelemetryReader) {
    let shared = Arc::new(SharedTelemetry::default());
    (
        TelemetryPublisher {
            shared: Arc::clone(&shared),
        },
        TelemetryReader { shared },
    )
}

struct SharedTelemetry {
    sequence: AtomicU64,
    left_peak: AtomicU32,
    left_rms: AtomicU32,
    right_peak: AtomicU32,
    right_rms: AtomicU32,
    spectrum: [AtomicU32; SPECTRUM_BAND_COUNT],
}

impl Default for SharedTelemetry {
    fn default() -> Self {
        Self {
            sequence: AtomicU64::new(0),
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
        let (publisher, reader) = channel();
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

        assert_eq!(reader.try_latest(), Some(TelemetryFrame::default()));
        publisher.publish(frame);
        assert_eq!(reader.try_latest(), Some(frame));
    }

    #[test]
    fn reader_does_not_wait_for_an_in_progress_publication() {
        let (publisher, reader) = channel();
        publisher.shared.sequence.store(1, Ordering::Release);

        assert_eq!(reader.try_latest(), None);
    }

    #[test]
    fn concurrent_publisher_does_not_write_into_an_owned_publication() {
        let (publisher, reader) = channel();
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
        publisher.publish(initial);
        let sequence = publisher.shared.sequence.load(Ordering::Acquire);
        publisher
            .shared
            .sequence
            .compare_exchange(sequence, sequence + 1, Ordering::AcqRel, Ordering::Acquire)
            .unwrap();

        let concurrent_publisher = publisher.clone();
        concurrent_publisher.publish(TelemetryFrame::default());
        publisher
            .shared
            .sequence
            .store(sequence + 2, Ordering::Release);

        assert_eq!(reader.try_latest(), Some(initial));
    }
}
