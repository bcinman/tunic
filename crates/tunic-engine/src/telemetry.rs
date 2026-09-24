use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

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

#[derive(Clone)]
pub struct TelemetryReader {
    shared: Arc<SharedLevels>,
}

impl TelemetryReader {
    #[must_use]
    pub fn latest(&self) -> StereoLevels {
        loop {
            let before = self.shared.sequence.load(Ordering::SeqCst);
            if !before.is_multiple_of(2) {
                std::hint::spin_loop();
                continue;
            }
            let levels = self.shared.load();
            let after = self.shared.sequence.load(Ordering::SeqCst);
            if before == after {
                return levels;
            }
        }
    }
}

#[derive(Clone)]
pub struct TelemetryPublisher {
    shared: Arc<SharedLevels>,
}

impl TelemetryPublisher {
    pub fn publish(&self, levels: StereoLevels) {
        self.shared.sequence.fetch_add(1, Ordering::SeqCst);
        self.shared.store(levels);
        self.shared.sequence.fetch_add(1, Ordering::SeqCst);
    }
}

pub(crate) fn channel() -> (TelemetryPublisher, TelemetryReader) {
    let shared = Arc::new(SharedLevels::default());
    (
        TelemetryPublisher {
            shared: Arc::clone(&shared),
        },
        TelemetryReader { shared },
    )
}

#[derive(Default)]
struct SharedLevels {
    sequence: AtomicU64,
    left_peak: AtomicU32,
    left_rms: AtomicU32,
    right_peak: AtomicU32,
    right_rms: AtomicU32,
}

impl SharedLevels {
    fn store(&self, levels: StereoLevels) {
        self.left_peak
            .store(levels.left.peak.to_bits(), Ordering::SeqCst);
        self.left_rms
            .store(levels.left.rms.to_bits(), Ordering::SeqCst);
        self.right_peak
            .store(levels.right.peak.to_bits(), Ordering::SeqCst);
        self.right_rms
            .store(levels.right.rms.to_bits(), Ordering::SeqCst);
    }

    fn load(&self) -> StereoLevels {
        StereoLevels {
            left: ChannelLevels {
                peak: f32::from_bits(self.left_peak.load(Ordering::SeqCst)),
                rms: f32::from_bits(self.left_rms.load(Ordering::SeqCst)),
            },
            right: ChannelLevels {
                peak: f32::from_bits(self.right_peak.load(Ordering::SeqCst)),
                rms: f32::from_bits(self.right_rms.load(Ordering::SeqCst)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ChannelLevels, StereoLevels, channel};

    #[test]
    fn reader_returns_the_latest_complete_publication() {
        let (publisher, reader) = channel();
        let levels = StereoLevels {
            left: ChannelLevels {
                peak: 0.75,
                rms: 0.5,
            },
            right: ChannelLevels {
                peak: 0.25,
                rms: 0.125,
            },
        };

        assert_eq!(reader.latest(), StereoLevels::default());
        publisher.publish(levels);
        assert_eq!(reader.latest(), levels);
    }
}
