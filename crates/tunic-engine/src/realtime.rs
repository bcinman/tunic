use std::num::NonZeroUsize;
use std::sync::Arc;

use tunic_dsp::{
    Analyzer, Equalizer, EqualizerError, GraphProcessor, GraphPublisher, PreparedGraph,
};

use crate::{
    BypassControl, ProcessedOutputSink, TelemetryFrame, TelemetryGeneration, TelemetryPublisher,
};

/// Publishes sample-rate-specific equalizer updates to a [`RealtimeProcessor`].
pub struct RealtimePublisher {
    graph: GraphPublisher,
    sample_rate_hz: f64,
}

impl RealtimePublisher {
    pub fn set_equalizer(&self, equalizer: &Equalizer) -> Result<(), EqualizerError> {
        self.graph
            .publish(PreparedGraph::prepare(equalizer, self.sample_rate_hz)?);
        Ok(())
    }
}

/// Applies portable real-time processing policy to normalized interleaved stereo samples.
///
/// `process` neither allocates, frees, nor locks. Platform callbacks provide normalized samples
/// and a writer for their native output buffers; telemetry and capture observe only samples that
/// the writer accepted.
pub struct RealtimeProcessor {
    bypass: BypassControl,
    was_bypassed: bool,
    graph: GraphProcessor,
    observers: OutputObservers,
}

impl RealtimeProcessor {
    pub fn new(
        equalizer: &Equalizer,
        sample_rate_hz: f64,
        frame_capacity: NonZeroUsize,
        bypass: BypassControl,
        output_sink: Option<Arc<dyn ProcessedOutputSink>>,
        telemetry: TelemetryPublisher,
    ) -> Result<(RealtimePublisher, Self), EqualizerError> {
        let prepared = PreparedGraph::prepare(equalizer, sample_rate_hz)?;
        let (graph, processor) = GraphProcessor::new(prepared, sample_rate_hz, frame_capacity)?;
        Ok((
            RealtimePublisher {
                graph,
                sample_rate_hz,
            },
            Self {
                bypass,
                was_bypassed: false,
                graph: processor,
                observers: OutputObservers::new(output_sink, telemetry, sample_rate_hz),
            },
        ))
    }

    pub fn process(
        &mut self,
        interleaved_stereo: &mut [f32],
        write: impl FnOnce(&[f32]) -> bool,
    ) -> bool {
        let bypassed = self.bypass.is_bypassed();
        if bypassed && !self.was_bypassed {
            self.graph.reset();
        }
        self.was_bypassed = bypassed;
        if !bypassed {
            self.graph
                .process(interleaved_stereo.as_chunks_mut::<2>().0);
        }
        let written = write(interleaved_stereo);
        if written {
            self.observers.observe(interleaved_stereo);
        }
        written
    }
}

struct OutputObservers {
    output_sink: Option<Arc<dyn ProcessedOutputSink>>,
    telemetry: TelemetryPublisher,
    telemetry_generation: Option<TelemetryGeneration>,
    analyzer: Analyzer,
}

impl OutputObservers {
    fn new(
        output_sink: Option<Arc<dyn ProcessedOutputSink>>,
        telemetry: TelemetryPublisher,
        sample_rate_hz: f64,
    ) -> Self {
        Self {
            output_sink,
            telemetry,
            telemetry_generation: None,
            analyzer: Analyzer::new(sample_rate_hz),
        }
    }

    fn observe(&mut self, samples: &[f32]) {
        if let Some(generation) = self.telemetry.active_generation() {
            if self.telemetry_generation != Some(generation) {
                self.analyzer.reset();
                self.telemetry_generation = Some(generation);
            }
            if let Some(frame) = self.analyzer.observe(samples) {
                self.telemetry.publish(
                    generation,
                    TelemetryFrame {
                        levels: frame.levels,
                        spectrum: frame.spectrum,
                    },
                );
            }
        } else {
            self.telemetry_generation = None;
        }
        if let Some(output_sink) = &self.output_sink {
            output_sink.write(samples);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use tunic_dsp::{Equalizer, Filter, FrequencyHz, GainDb, QualityFactor};

    use super::RealtimeProcessor;
    use crate::BypassControl;
    use crate::telemetry::channel;

    #[test]
    fn entering_bypass_clears_filter_history() {
        let (telemetry, _) = channel();
        let bypass = BypassControl::default();
        let (_, mut processor) = RealtimeProcessor::new(
            &Equalizer::with_filter(Filter::peaking(
                FrequencyHz::new(1_000.0).unwrap(),
                GainDb::new(12.0).unwrap(),
                QualityFactor::new(1.0).unwrap(),
            )),
            48_000.0,
            NonZeroUsize::new(64).unwrap(),
            bypass.clone(),
            None,
            telemetry,
        )
        .unwrap();
        let mut impulse = [1.0_f32, 1.0];
        processor.process(&mut impulse, |_| true);

        bypass.toggle();
        let mut silence = [0.0_f32, 0.0];
        processor.process(&mut silence, |_| true);
        bypass.toggle();
        processor.process(&mut silence, |_| true);

        assert_eq!(silence, [0.0, 0.0]);
    }

    #[test]
    fn writer_rejection_prevents_output_observation() {
        let (telemetry, source) = channel();
        let reader = source.subscribe();
        let (_, mut processor) = RealtimeProcessor::new(
            &Equalizer::identity(),
            48_000.0,
            NonZeroUsize::new(4_096).unwrap(),
            BypassControl::default(),
            None,
            telemetry,
        )
        .unwrap();
        let mut samples = vec![1.0; 4_096 * 2];

        assert!(!processor.process(&mut samples, |_| false));

        assert_eq!(reader.try_latest(), None);
    }
}
