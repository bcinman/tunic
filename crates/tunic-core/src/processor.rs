use std::num::NonZeroUsize;

use crate::{Chain, dsp::PreparedChain};
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
    chain: PreparedChain,
    bypassed: bool,
    maximum_frame_count: NonZeroUsize,
}

/// Publishes non-real-time chain and bypass updates to a [`Processor`].
#[derive(Clone)]
pub struct Controller {
    _private: (),
}

/// A read-only latest-value view of measurements produced by a [`Processor`].
#[derive(Clone)]
pub struct Telemetry {
    _private: (),
}

impl Processor {
    pub fn new(
        format: AudioFormat,
        chain: Chain,
        bypassed: bool,
    ) -> Result<(Self, Controller), ProcessorError> {
        let chain = PreparedChain::prepare(&chain, format.sample_rate)?;
        Ok((
            Self {
                chain,
                bypassed,
                maximum_frame_count: format.maximum_frame_count,
            },
            Controller { _private: () },
        ))
    }

    pub fn process(&mut self, interleaved_stereo: &mut [f32]) {
        debug_assert_eq!(interleaved_stereo.len() % 2, 0);
        debug_assert!(interleaved_stereo.len() / 2 <= self.maximum_frame_count.get());
        if !self.bypassed {
            self.chain.process(interleaved_stereo);
        }
    }
}

#[allow(clippy::todo)]
impl Controller {
    pub fn set_chain(&self, _chain: Chain) -> Result<(), ProcessorError> {
        todo!()
    }

    pub fn set_bypassed(&self, _bypassed: bool) {
        todo!()
    }

    pub fn is_bypassed(&self) -> bool {
        todo!()
    }

    pub fn subscribe_telemetry(&self) -> Telemetry {
        todo!()
    }
}

#[allow(clippy::todo)]
impl Telemetry {
    pub fn latest(&self) -> Option<TelemetryFrame> {
        todo!()
    }
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
    PreampOutOfRange,
    FilterAtOrAboveNyquist { filter: usize },
    UnstableFilter { filter: usize },
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::{AudioFormat, Processor, SampleRateHz};
    use crate::{Chain, Equalizer, GainDb};

    #[test]
    fn sample_rates_are_positive_and_finite() {
        assert!(SampleRateHz::try_new(0.0).is_err());
        assert!(SampleRateHz::try_new(f64::NAN).is_err());
        assert!(SampleRateHz::try_new(48_000.0).is_ok());
    }

    #[test]
    fn processor_applies_its_initial_chain_unless_bypassed() {
        let chain = Chain {
            equalizer: Equalizer {
                preamp: GainDb::try_new(6.0).unwrap(),
                filters: Vec::new(),
            },
        };
        let format = AudioFormat {
            sample_rate: SampleRateHz::try_new(48_000.0).unwrap(),
            maximum_frame_count: NonZeroUsize::new(2).unwrap(),
        };
        let (mut active, _) = Processor::new(format, chain.clone(), false).unwrap();
        let (mut bypassed, _) = Processor::new(format, chain, true).unwrap();
        let mut active_samples = [0.25, -0.5, 0.125, -0.25];
        let mut bypassed_samples = active_samples;

        active.process(&mut active_samples);
        bypassed.process(&mut bypassed_samples);

        let gain = 10.0_f32.powf(6.0 / 20.0);
        assert!((active_samples[0] - 0.25 * gain).abs() < f32::EPSILON);
        assert!((active_samples[1] + 0.5 * gain).abs() < f32::EPSILON);
        assert_eq!(bypassed_samples, [0.25, -0.5, 0.125, -0.25]);
    }
}
