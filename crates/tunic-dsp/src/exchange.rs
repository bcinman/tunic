use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::{EqualizerError, PreparedGraph};

const GRAPH_CROSSFADE_SECONDS: f64 = 0.005;

/// Publishes prepared graphs to a [`GraphProcessor`] without blocking its audio thread.
#[derive(Clone)]
pub struct GraphPublisher {
    exchange: Arc<GraphExchange>,
}

impl GraphPublisher {
    /// Makes `graph` the latest graph waiting for the real-time processor.
    ///
    /// Superseded graphs are reclaimed on this non-real-time caller.
    pub fn publish(&self, graph: PreparedGraph) {
        self.exchange.publish(graph);
    }
}

/// Processes audio through the latest published graph and smooths graph replacements.
///
/// `process` and `reset` neither allocate, free, nor lock. A processor is intended to be owned
/// by one real-time audio callback, while its cloneable [`GraphPublisher`] is used off-thread.
pub struct GraphProcessor {
    current: PreparedGraph,
    fading_from: *mut GraphNode,
    crossfade_frame: usize,
    crossfade_frames: usize,
    crossfade_scratch: Vec<[f32; 2]>,
    exchange: Arc<GraphExchange>,
}

// SAFETY: the detached node is exclusively owned by the processor. Moving the processor transfers
// that ownership, and all access still requires `&mut self`.
unsafe impl Send for GraphProcessor {}

impl GraphProcessor {
    pub fn new(
        current: PreparedGraph,
        sample_rate_hz: f64,
        frame_capacity: NonZeroUsize,
    ) -> Result<(GraphPublisher, Self), EqualizerError> {
        if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
            return Err(EqualizerError::new(
                "sample rate must be finite and greater than zero",
            ));
        }
        let exchange = Arc::new(GraphExchange::new());
        Ok((
            GraphPublisher {
                exchange: Arc::clone(&exchange),
            },
            Self {
                current,
                fading_from: std::ptr::null_mut(),
                crossfade_frame: 0,
                crossfade_frames: (sample_rate_hz * GRAPH_CROSSFADE_SECONDS).round().max(1.0)
                    as usize,
                crossfade_scratch: vec![[0.0; 2]; frame_capacity.get()],
                exchange,
            },
        ))
    }

    /// Processes stereo frames in place through the latest published graph.
    pub fn process(&mut self, frames: &mut [[f32; 2]]) {
        self.install_latest();
        let mut processed = 0;
        while processed < frames.len() {
            if self.fading_from.is_null() {
                self.current.process(&mut frames[processed..]);
                return;
            }

            let chunk_frames = (frames.len() - processed).min(self.crossfade_scratch.len());
            let current_frames = &mut frames[processed..processed + chunk_frames];
            let previous_frames = &mut self.crossfade_scratch[..chunk_frames];
            previous_frames.copy_from_slice(current_frames);
            self.current.process(current_frames);
            // SAFETY: `fading_from` is detached from the exchange and owned by this processor.
            unsafe { &mut (*self.fading_from).graph }.process(previous_frames);
            for (offset, (current, previous)) in current_frames
                .iter_mut()
                .zip(previous_frames.iter())
                .enumerate()
            {
                let mix = ((self.crossfade_frame + offset + 1) as f32
                    / self.crossfade_frames as f32)
                    .min(1.0);
                current[0] = previous[0] + (current[0] - previous[0]) * mix;
                current[1] = previous[1] + (current[1] - previous[1]) * mix;
            }
            self.crossfade_frame += chunk_frames;
            processed += chunk_frames;
            if self.crossfade_frame >= self.crossfade_frames {
                self.exchange.retire(self.fading_from);
                self.fading_from = std::ptr::null_mut();
            }
        }
    }

    pub fn reset(&mut self) {
        self.current.reset();
        if !self.fading_from.is_null() {
            // SAFETY: this processor exclusively owns the detached node.
            unsafe { &mut (*self.fading_from).graph }.reset();
        }
    }

    fn install_latest(&mut self) {
        if !self.fading_from.is_null() {
            return;
        }
        let update = self.exchange.take_latest();
        if update.is_null() {
            return;
        }
        // SAFETY: this processor won ownership of the pending node in `take_latest`.
        let node = unsafe { &mut *update };
        std::mem::swap(&mut self.current, &mut node.graph);
        self.fading_from = update;
        self.crossfade_frame = 0;
    }
}

impl Drop for GraphProcessor {
    fn drop(&mut self) {
        if !self.fading_from.is_null() {
            // SAFETY: this detached node belongs to the processor rather than the exchange lists.
            drop(unsafe { Box::from_raw(self.fading_from) });
        }
    }
}

struct GraphExchange {
    pending: AtomicPtr<GraphNode>,
    retired: AtomicPtr<GraphNode>,
}

impl GraphExchange {
    fn new() -> Self {
        Self {
            pending: AtomicPtr::new(std::ptr::null_mut()),
            retired: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    fn publish(&self, graph: PreparedGraph) {
        self.reclaim_retired();
        let update = Box::into_raw(Box::new(GraphNode {
            graph,
            next: std::ptr::null_mut(),
        }));
        let superseded = self.pending.swap(update, Ordering::AcqRel);
        if !superseded.is_null() {
            // SAFETY: the producer won ownership of the pending node in the swap.
            drop(unsafe { Box::from_raw(superseded) });
        }
        self.reclaim_retired();
    }

    fn take_latest(&self) -> *mut GraphNode {
        self.pending.swap(std::ptr::null_mut(), Ordering::AcqRel)
    }

    fn retire(&self, node: *mut GraphNode) {
        let mut head = self.retired.load(Ordering::Acquire);
        loop {
            // SAFETY: the processor exclusively owns `node` until this exchange succeeds.
            unsafe { (*node).next = head };
            match self.retired.compare_exchange_weak(
                head,
                node,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(actual) => head = actual,
            }
        }
    }

    fn reclaim_retired(&self) {
        let retired = self.retired.swap(std::ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: the producer owns every node in the detached retired list.
        unsafe { drop_nodes(retired) };
    }
}

impl Drop for GraphExchange {
    fn drop(&mut self) {
        unsafe {
            drop_nodes(*self.pending.get_mut());
            drop_nodes(*self.retired.get_mut());
        }
    }
}

struct GraphNode {
    graph: PreparedGraph,
    next: *mut GraphNode,
}

unsafe fn drop_nodes(mut node: *mut GraphNode) {
    while !node.is_null() {
        // SAFETY: the caller owns every node in this detached list.
        let boxed = unsafe { Box::from_raw(node) };
        node = boxed.next;
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use crate::{Equalizer, Filter, FrequencyHz, GainDb, PreparedGraph, QualityFactor};

    use super::GraphProcessor;

    #[test]
    fn exchange_handles_have_callback_compatible_thread_traits() {
        fn assert_send<T: Send>() {}
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send::<GraphProcessor>();
        assert_send_sync::<super::GraphPublisher>();
    }

    #[test]
    fn processor_installs_only_the_latest_pending_graph() {
        let (publisher, mut processor) = GraphProcessor::new(
            PreparedGraph::identity(),
            48_000.0,
            NonZeroUsize::new(64).unwrap(),
        )
        .unwrap();
        publisher.publish(peaking_graph(6.0));
        publisher.publish(peaking_graph(-6.0));
        let mut silence = [[0.0, 0.0]];

        processor.process(&mut silence);

        assert!((processor.current.response_db_at(1_000.0) + 6.0).abs() < 0.001);
    }

    #[test]
    fn graph_update_starts_with_the_previous_output_and_crossfades() {
        let (publisher, mut processor) =
            GraphProcessor::new(peaking_graph(6.0), 48_000.0, NonZeroUsize::new(64).unwrap())
                .unwrap();
        publisher.publish(peaking_graph(-6.0));
        let mut frames = [[1.0_f32, 1.0]];
        let mut previous = peaking_graph(6.0);
        let mut previous_output = frames;
        previous.process(&mut previous_output);
        let mut next = peaking_graph(-6.0);
        let mut next_output = frames;
        next.process(&mut next_output);

        processor.process(&mut frames);

        let expected = previous_output[0][0]
            + (next_output[0][0] - previous_output[0][0]) / processor.crossfade_frames as f32;
        assert!((frames[0][0] - expected).abs() < 1e-6);
        assert_eq!(frames[0][0], frames[0][1]);
        assert!(!processor.fading_from.is_null());
    }

    #[test]
    fn active_crossfade_finishes_before_installing_the_latest_update() {
        let (publisher, mut processor) =
            GraphProcessor::new(peaking_graph(6.0), 48_000.0, NonZeroUsize::new(64).unwrap())
                .unwrap();
        publisher.publish(peaking_graph(-6.0));
        processor.process(&mut [[0.25_f32, -0.5]; 64]);
        publisher.publish(peaking_graph(12.0));

        processor.process(&mut [[0.25_f32, -0.5]; 64]);

        assert!((processor.current.response_db_at(1_000.0) + 6.0).abs() < 0.001);
        assert_eq!(processor.crossfade_frame, 128);

        processor.process(&mut [[0.25_f32, -0.5]; 112]);
        assert!(processor.fading_from.is_null());
        processor.process(&mut [[0.25_f32, -0.5]; 1]);
        assert!((processor.current.response_db_at(1_000.0) - 12.0).abs() < 0.001);
    }

    #[test]
    fn processor_chunks_buffers_larger_than_its_transition_scratch() {
        let (publisher, mut processor) =
            GraphProcessor::new(peaking_graph(6.0), 48_000.0, NonZeroUsize::new(7).unwrap())
                .unwrap();
        publisher.publish(peaking_graph(-6.0));
        let mut frames = vec![[0.25_f32, -0.5]; 300];
        let mut expected = frames.clone();
        peaking_graph(-6.0).process(&mut expected);

        processor.process(&mut frames);

        assert_eq!(
            &frames[processor.crossfade_frames..],
            &expected[processor.crossfade_frames..]
        );
        assert!(processor.fading_from.is_null());
    }

    #[test]
    fn invalid_sample_rate_is_rejected_before_processing() {
        assert!(
            GraphProcessor::new(
                PreparedGraph::identity(),
                0.0,
                NonZeroUsize::new(64).unwrap()
            )
            .is_err()
        );
    }

    fn peaking_graph(gain_db: f64) -> PreparedGraph {
        let equalizer = Equalizer::with_filter(Filter::peaking(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(gain_db).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        ));
        PreparedGraph::prepare(&equalizer, 48_000.0).unwrap()
    }
}
