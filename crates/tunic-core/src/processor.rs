use std::num::NonZeroUsize;

use nutype::nutype;

pub const SPECTRUM_POINT_COUNT: usize = 256;

#[nutype(
    validate(finite, greater = 0.0),
    derive(Clone, Copy, Debug, PartialEq, PartialOrd, TryFrom)
)]
pub struct SampleRateHz(f64);

/// The normalized stream format accepted by a [`Processor`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioFormat {
    pub sample_rate: SampleRateHz,
    pub maximum_frame_count: NonZeroUsize,
}

/// A stateful processor exclusively owned by an audio callback.
///
/// Its implementation will guarantee that processing does not allocate, lock,
/// or block.
pub struct Processor {
    _private: (),
}

/// A read-only latest-value view of measurements produced by a [`Processor`].
pub struct Telemetry {
    _private: (),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TelemetryFrame {
    pub levels: StereoLevels,
    pub spectrum: Spectrum,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StereoLevels {
    pub left: ChannelLevels,
    pub right: ChannelLevels,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChannelLevels {
    pub peak: f32,
    pub rms: f32,
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessorError {
    FilterAtOrAboveNyquist { filter: usize },
}

#[cfg(test)]
mod tests {
    use super::SampleRateHz;

    #[test]
    fn sample_rates_are_positive_and_finite() {
        assert!(SampleRateHz::try_new(0.0).is_err());
        assert!(SampleRateHz::try_new(f64::NAN).is_err());
        assert!(SampleRateHz::try_new(48_000.0).is_ok());
    }
}
