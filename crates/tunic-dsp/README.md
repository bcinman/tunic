# `tunic-dsp`

**Responsibility:** Portable audio-processing definitions and algorithms.

- Defines and validates the equalizer; versioning and
  serialization are planned.
- Rejects invalid filter parameters and sample-rate-specific equalizers.
- Prepares processing graphs for a specific sample rate.
- Processes arbitrary stereo frame counts without allocation.
- Supports reset, bypass integration, and latency reporting.
- Contains mathematical, impulse-response, and frequency-response tests.
- Knows nothing about devices, GPUI, profiles, persistence, or Core Audio.

## Rough public interface

The current implementation supports identity processing or an ordered cascade
of peaking filters. Versioning, serialization, and additional filter types
remain planned.

```rust
pub struct Equalizer;
pub struct PeakingFilter;
pub struct PreparedGraph;

impl Equalizer {
    pub fn identity() -> Self;
    pub fn with_peaking_filter(filter: PeakingFilter) -> Self;
    pub fn with_peaking_filters(filters: Vec<PeakingFilter>) -> Self;
    pub fn peaking_filters(&self) -> &[PeakingFilter];
}

impl PreparedGraph {
    pub fn prepare(
        equalizer: &Equalizer,
        sample_rate_hz: f64,
    ) -> Result<Self, EqualizerError>;

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
