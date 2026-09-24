# `tunic-dsp`

**Responsibility:** Portable audio-processing definitions and algorithms.

- Defines and validates the processing configuration; versioning and
  serialization are planned.
- Rejects invalid filter parameters and sample-rate-specific configurations.
- Prepares processing graphs for a specific sample rate.
- Processes arbitrary stereo frame counts without allocation.
- Supports reset, bypass integration, and latency reporting.
- Contains mathematical, impulse-response, and frequency-response tests.
- Knows nothing about devices, GPUI, profiles, persistence, or Core Audio.

## Rough public interface

The current implementation supports either identity processing or one peaking
filter. Versioning, serialization, and additional filter types remain planned.

```rust
pub struct Configuration;
pub struct PeakingFilter;
pub struct PreparedGraph;

impl Configuration {
    pub fn identity() -> Self;
    pub fn with_peaking_filter(filter: PeakingFilter) -> Self;
}

impl PreparedGraph {
    pub fn prepare(
        configuration: &Configuration,
        sample_rate_hz: f64,
    ) -> Result<Self, ConfigurationError>;

    pub fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
    );

    pub fn reset(&mut self);
    pub fn latency_frames(&self) -> usize;
}
```

See the [workspace structure](../../STRUCTURE.md) for the complete crate layout and dependency direction.
