//! Portable audio-processing definitions and algorithms.

/// A processing graph prepared for a concrete audio stream.
///
/// The first implementation is deliberately an identity graph. It establishes the
/// real-time processing boundary before Tunic adds filters.
#[derive(Debug, Default)]
pub struct PreparedGraph;

impl PreparedGraph {
    #[must_use]
    pub fn identity() -> Self {
        Self
    }

    /// Process stereo audio in place without allocating.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
    }

    /// Process interleaved stereo frames in place without allocating.
    pub fn process_interleaved_stereo(&mut self, samples: &mut [f32]) {
        debug_assert_eq!(samples.len() % 2, 0);
    }

    pub fn reset(&mut self) {}

    #[must_use]
    pub fn latency_frames(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::PreparedGraph;

    #[test]
    fn identity_graph_preserves_asymmetric_stereo_samples() {
        let mut graph = PreparedGraph::identity();
        let mut left = [0.25, -0.75, 1.0];
        let mut right = [-0.5, 0.125, -1.0];

        graph.process(&mut left, &mut right);

        assert_eq!(left, [0.25, -0.75, 1.0]);
        assert_eq!(right, [-0.5, 0.125, -1.0]);

        let mut interleaved = [0.25, -0.5, -0.75, 0.125, 1.0, -1.0];
        graph.process_interleaved_stereo(&mut interleaved);
        assert_eq!(interleaved, [0.25, -0.5, -0.75, 0.125, 1.0, -1.0]);
        assert_eq!(graph.latency_frames(), 0);
    }
}
