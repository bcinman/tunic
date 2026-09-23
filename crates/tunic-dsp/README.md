# `tunic-dsp`

**Responsibility:** Portable audio-processing definitions and algorithms.

- Defines the versioned processing configuration.
- Validates and canonicalizes configurations.
- Prepares processing graphs for a specific sample rate.
- Processes arbitrary stereo frame counts without allocation.
- Supports reset, bypass integration, and latency reporting.
- Contains mathematical, impulse-response, and frequency-response tests.
- Knows nothing about devices, GPUI, profiles, persistence, or Core Audio.

## Rough public interface

```rust
pub struct Configuration;
pub struct PreparedGraph;

impl Configuration {
    pub fn parse_json(input: &str) -> Result<Self, ConfigurationError>;
    pub fn to_canonical_json(&self) -> Result<String, ConfigurationError>;
}

impl PreparedGraph {
    pub fn prepare(
        configuration: &Configuration,
        sample_rate_hz: f64,
    ) -> Result<Self, PrepareError>;

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
