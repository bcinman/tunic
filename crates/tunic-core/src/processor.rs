//! Public real-time processing and non-real-time control boundary.
//!
//! The processor owns callback-local DSP state. Its cloneable controller
//! prepares chain replacements and publishes chain and bypass changes.

mod analyzer;
mod dsp;
mod exchange;
mod telemetry;

pub use telemetry::{
    ChannelLevels, SPECTRUM_MAX_FREQUENCY_HZ, SPECTRUM_MIN_FREQUENCY_HZ, SPECTRUM_POINT_COUNT,
    Spectrum, StereoLevels, Telemetry, TelemetryFrame, spectrum_frequency_hz,
};

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use self::{
    analyzer::Analyzer,
    dsp::PreparedChain,
    exchange::{ChainPublisher, ChainReader},
    telemetry::{TelemetryGeneration, TelemetryPublisher, TelemetrySource},
};
use crate::Chain;
use nutype::nutype;

const CHAIN_CROSSFADE_SECONDS: f64 = 0.005;

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
/// or block. Drop it only after the callback has stopped and on a non-real-time
/// thread; destruction may free prepared chains and scratch storage.
pub struct Processor {
    current: PreparedChain,
    chain_updates: ChainReader,
    crossfade_frame: Option<usize>,
    crossfade_frames: usize,
    crossfade_scratch: Vec<[f32; 2]>,
    bypassed: Arc<AtomicBool>,
    was_bypassed: bool,
    maximum_frame_count: NonZeroUsize,
    analyzer: Analyzer,
    telemetry: TelemetryPublisher,
    telemetry_generation: Option<TelemetryGeneration>,
}

/// Publishes non-real-time chain and bypass updates to a [`Processor`].
#[derive(Clone)]
pub struct Controller {
    chain_updates: ChainPublisher,
    bypassed: Arc<AtomicBool>,
    sample_rate: SampleRateHz,
    telemetry: TelemetrySource,
}

/// A prepared, read-only view of a chain's combined filter response.
///
/// Preamp gain is intentionally excluded so filter control points remain relative
/// to the equalizer's zero line. Preparation computes filter coefficients once;
/// callers can then sample as many graph frequencies as needed.
pub struct FrequencyResponse {
    prepared: dsp::PreparedResponse,
}

impl FrequencyResponse {
    pub fn new(chain: &Chain, sample_rate: SampleRateHz) -> Result<Self, ProcessorError> {
        Ok(Self {
            prepared: dsp::PreparedResponse::prepare(chain, sample_rate)?,
        })
    }

    pub fn db_at(&self, frequency: crate::FrequencyHz) -> Result<f64, ProcessorError> {
        self.prepared.db_at(frequency)
    }
}

impl Processor {
    pub fn new(
        format: AudioFormat,
        chain: Chain,
        bypassed: bool,
    ) -> Result<(Self, Controller), ProcessorError> {
        let chain = PreparedChain::prepare(&chain, format.sample_rate)?;
        let (chain_updates, chain_publisher) = exchange::channel();
        let (telemetry_publisher, telemetry_source) = telemetry::channel();
        let bypassed = Arc::new(AtomicBool::new(bypassed));
        Ok((
            Self {
                current: chain,
                chain_updates,
                crossfade_frame: None,
                crossfade_frames: (format.sample_rate.into_inner() * CHAIN_CROSSFADE_SECONDS)
                    .round()
                    .max(1.0) as usize,
                crossfade_scratch: vec![[0.0; 2]; format.maximum_frame_count.get()],
                bypassed: Arc::clone(&bypassed),
                was_bypassed: bypassed.load(Ordering::Relaxed),
                maximum_frame_count: format.maximum_frame_count,
                analyzer: Analyzer::new(format.sample_rate),
                telemetry: telemetry_publisher,
                telemetry_generation: None,
            },
            Controller {
                chain_updates: chain_publisher,
                bypassed,
                sample_rate: format.sample_rate,
                telemetry: telemetry_source,
            },
        ))
    }

    /// Processes complete interleaved stereo frames up to the configured frame capacity.
    pub fn process(&mut self, interleaved_stereo: &mut [f32]) {
        let (frames, remainder) = interleaved_stereo.as_chunks_mut::<2>();
        debug_assert!(remainder.is_empty());
        debug_assert!(frames.len() <= self.maximum_frame_count.get());

        let bypassed = self.bypassed.load(Ordering::Relaxed);
        if bypassed && !self.was_bypassed {
            self.reset();
        }
        self.was_bypassed = bypassed;
        if !bypassed {
            self.process_active(frames);
        }
        self.observe_telemetry(frames);
    }

    fn process_active(&mut self, frames: &mut [[f32; 2]]) {
        if self.crossfade_frame.is_none() && self.chain_updates.try_adopt(&mut self.current) {
            self.crossfade_frame = Some(0);
        }
        let Some(crossfade_frame) = self.crossfade_frame else {
            self.current.process(frames);
            return;
        };

        let transition_frames = frames.len().min(self.crossfade_frames - crossfade_frame);
        let (transition, settled) = frames.split_at_mut(transition_frames);
        let previous = &mut self.crossfade_scratch[..transition_frames];
        previous.copy_from_slice(transition);
        self.current.process(transition);
        self.chain_updates.previous_mut().process(previous);
        for (offset, (current, previous)) in transition.iter_mut().zip(previous.iter()).enumerate()
        {
            let mix = (crossfade_frame + offset + 1) as f32 / self.crossfade_frames as f32;
            current[0] = previous[0] + (current[0] - previous[0]) * mix;
            current[1] = previous[1] + (current[1] - previous[1]) * mix;
        }

        let crossfade_frame = crossfade_frame + transition_frames;
        self.crossfade_frame = (crossfade_frame < self.crossfade_frames).then_some(crossfade_frame);
        if !settled.is_empty() {
            self.current.process(settled);
        }
    }

    fn reset(&mut self) {
        self.current.reset();
        if self.crossfade_frame.is_some() {
            self.chain_updates.previous_mut().reset();
        }
    }

    fn observe_telemetry(&mut self, frames: &[[f32; 2]]) {
        let Some(generation) = self.telemetry.active_generation() else {
            self.telemetry_generation = None;
            return;
        };
        if self.telemetry_generation != Some(generation) {
            self.analyzer.reset();
            self.telemetry_generation = Some(generation);
        }
        if let Some(frame) = self.analyzer.observe(frames) {
            self.telemetry.publish(generation, frame);
        }
    }
}

impl Controller {
    #[must_use]
    pub fn sample_rate(&self) -> SampleRateHz {
        self.sample_rate
    }

    pub fn set_chain(&self, chain: Chain) -> Result<(), ProcessorError> {
        let chain = PreparedChain::prepare(&chain, self.sample_rate)?;
        self.chain_updates.publish(chain);
        Ok(())
    }

    pub fn set_bypassed(&self, bypassed: bool) {
        self.bypassed.store(bypassed, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_bypassed(&self) -> bool {
        self.bypassed.load(Ordering::Relaxed)
    }

    pub fn subscribe_telemetry(&self) -> Telemetry {
        self.telemetry.subscribe()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessorError {
    PreampOutOfRange,
    ResponseAtOrAboveNyquist,
    FilterAtOrAboveNyquist { filter: usize },
    UnstableFilter { filter: usize },
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::{AudioFormat, Controller, Processor, ProcessorError, SampleRateHz};
    use crate::{Chain, Equalizer, Filter, FilterKind, FrequencyHz, GainDb, QualityFactor};

    #[test]
    fn sample_rates_are_positive_and_finite() {
        assert!(SampleRateHz::try_new(0.0).is_err());
        assert!(SampleRateHz::try_new(f64::NAN).is_err());
        assert!(SampleRateHz::try_new(48_000.0).is_ok());
    }

    #[test]
    fn processor_applies_its_initial_chain_unless_bypassed() {
        let chain = preamp(6.0);
        let format = format(2);
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

    #[test]
    fn processor_and_controller_have_the_required_thread_traits() {
        fn assert_send<T: Send>() {}
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send::<Processor>();
        assert_send_sync::<Controller>();
    }

    #[test]
    fn controller_publishes_only_the_latest_pending_chain() {
        let (mut processor, controller) = Processor::new(format(240), preamp(0.0), false).unwrap();
        controller.set_chain(preamp(6.0)).unwrap();
        controller.set_chain(preamp(-6.0)).unwrap();
        let mut transition = vec![1.0; 240 * 2];

        processor.process(&mut transition);
        let mut settled = [1.0, -0.5];
        processor.process(&mut settled);

        let gain = db_gain(-6.0);
        assert!((settled[0] - gain).abs() < f32::EPSILON);
        assert!((settled[1] + 0.5 * gain).abs() < f32::EPSILON);
    }

    #[test]
    fn chain_replacement_crossfades_from_the_previous_output() {
        let (mut processor, controller) = Processor::new(format(1), preamp(0.0), false).unwrap();
        controller.set_chain(preamp(6.0)).unwrap();
        let mut frame = [1.0, -0.5];

        processor.process(&mut frame);

        let mix = 1.0 / processor.crossfade_frames as f32;
        let gain = db_gain(6.0);
        assert!((frame[0] - (1.0 + (gain - 1.0) * mix)).abs() < f32::EPSILON);
        assert!((frame[1] - (-0.5 + (-0.5 * gain + 0.5) * mix)).abs() < f32::EPSILON);
        assert_eq!(processor.crossfade_frame, Some(1));
    }

    #[test]
    fn active_crossfade_finishes_before_installing_the_latest_update() {
        let (mut processor, controller) = Processor::new(format(176), preamp(0.0), false).unwrap();
        controller.set_chain(preamp(6.0)).unwrap();
        processor.process(&mut [1.0; 64 * 2]);
        controller.set_chain(preamp(-3.0)).unwrap();
        controller.set_chain(preamp(-6.0)).unwrap();

        processor.process(&mut [1.0; 176 * 2]);
        let mut frame = [1.0, 1.0];
        processor.process(&mut frame);

        let previous = db_gain(6.0);
        let next = db_gain(-6.0);
        let expected = previous + (next - previous) / processor.crossfade_frames as f32;
        assert!((frame[0] - expected).abs() < 1e-6);
        assert_eq!(frame[0], frame[1]);
    }

    #[test]
    fn invalid_chain_update_leaves_the_current_chain_unchanged() {
        let (mut processor, controller) = Processor::new(format(1), preamp(0.0), false).unwrap();

        assert_eq!(
            controller.set_chain(invalid_chain()),
            Err(ProcessorError::FilterAtOrAboveNyquist { filter: 0 })
        );
        let mut frame = [0.25, -0.5];
        processor.process(&mut frame);
        assert_eq!(frame, [0.25, -0.5]);
    }

    #[test]
    fn invalid_update_does_not_replace_a_valid_pending_update() {
        let (mut processor, controller) = Processor::new(format(240), preamp(0.0), false).unwrap();
        controller.set_chain(preamp(6.0)).unwrap();
        assert!(controller.set_chain(invalid_chain()).is_err());

        processor.process(&mut [1.0; 240 * 2]);
        let mut settled = [1.0, 1.0];
        processor.process(&mut settled);

        assert!((settled[0] - db_gain(6.0)).abs() < 1e-6);
    }

    #[test]
    fn pending_update_survives_its_controller_being_dropped() {
        let (mut processor, controller) = Processor::new(format(240), preamp(0.0), false).unwrap();
        controller.set_chain(preamp(-6.0)).unwrap();
        drop(controller);

        processor.process(&mut [1.0; 240 * 2]);
        let mut settled = [1.0, 1.0];
        processor.process(&mut settled);

        assert!((settled[0] - db_gain(-6.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn controller_can_outlive_the_processor() {
        let (processor, controller) = Processor::new(format(1), preamp(0.0), false).unwrap();
        drop(processor);

        controller.set_chain(preamp(6.0)).unwrap();
        controller.set_chain(preamp(-6.0)).unwrap();
    }

    #[test]
    fn publication_and_processing_can_run_concurrently() {
        let (mut processor, controller) = Processor::new(format(64), preamp(0.0), false).unwrap();
        let publisher = controller.clone();
        let processing = std::thread::spawn(move || {
            for _ in 0..200 {
                processor.process(&mut [0.25; 64 * 2]);
            }
            processor
        });
        let publishing = std::thread::spawn(move || {
            for index in 0..200 {
                let gain = if index % 2 == 0 { 6.0 } else { -6.0 };
                publisher.set_chain(preamp(gain)).unwrap();
            }
        });

        publishing.join().unwrap();
        let mut processor = processing.join().unwrap();
        controller.set_chain(preamp(3.0)).unwrap();
        for _ in 0..8 {
            processor.process(&mut [1.0; 64 * 2]);
        }
        let mut settled = [1.0, 1.0];
        processor.process(&mut settled);

        assert!((settled[0] - db_gain(3.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn bypass_is_immediate_and_clears_filter_history() {
        let chain = Chain {
            equalizer: Equalizer {
                preamp: GainDb::default(),
                filters: vec![filter(1_000.0, 12.0)],
            },
        };
        let (mut processor, controller) = Processor::new(format(1), chain, false).unwrap();
        let mut impulse = [1.0, 1.0];
        processor.process(&mut impulse);

        controller.set_bypassed(true);
        assert!(controller.is_bypassed());
        let mut bypassed = [0.25, -0.5];
        processor.process(&mut bypassed);
        assert_eq!(bypassed, [0.25, -0.5]);

        controller.set_bypassed(false);
        assert!(!controller.is_bypassed());
        let mut silence = [0.0, 0.0];
        processor.process(&mut silence);
        assert_eq!(silence, [0.0, 0.0]);
    }

    #[test]
    fn telemetry_observes_final_output_only_while_subscribed() {
        let (mut processor, controller) =
            Processor::new(format(4_096), preamp(6.0), false).unwrap();
        processor.process(&mut [0.25; 4_095 * 2]);
        let telemetry = controller.subscribe_telemetry();

        processor.process(&mut [0.25, -0.5]);
        assert_eq!(telemetry.latest(), None);

        processor.process(&mut [0.25; 4_095 * 2]);
        let frame = telemetry.latest().unwrap();
        let gain = db_gain(6.0);
        assert!((frame.levels.left.peak - 0.25 * gain).abs() < 1e-6);
        assert!((frame.levels.left.rms - 0.25 * gain).abs() < 1e-6);
    }

    fn format(maximum_frame_count: usize) -> AudioFormat {
        AudioFormat {
            sample_rate: SampleRateHz::try_new(48_000.0).unwrap(),
            maximum_frame_count: NonZeroUsize::new(maximum_frame_count).unwrap(),
        }
    }

    fn preamp(gain: f64) -> Chain {
        Chain {
            equalizer: Equalizer {
                preamp: GainDb::try_new(gain).unwrap(),
                filters: Vec::new(),
            },
        }
    }

    fn filter(frequency: f64, gain: f64) -> Filter {
        Filter {
            id: crate::FilterId::try_new(1).unwrap(),
            kind: FilterKind::Peaking,
            frequency: FrequencyHz::try_new(frequency).unwrap(),
            gain: GainDb::try_new(gain).unwrap(),
            quality_factor: QualityFactor::try_new(1.0).unwrap(),
        }
    }

    fn invalid_chain() -> Chain {
        Chain {
            equalizer: Equalizer {
                preamp: GainDb::default(),
                filters: vec![filter(24_000.0, 6.0)],
            },
        }
    }

    fn db_gain(gain: f32) -> f32 {
        10.0_f32.powf(gain / 20.0)
    }
}
