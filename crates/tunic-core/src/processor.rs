pub const SPECTRUM_POINT_COUNT: usize = 256;

/// The normalized stream format accepted by a [`Processor`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioFormat {
    pub sample_rate_hz: f64,
    pub maximum_frame_count: usize,
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
pub struct ProcessorError {
    pub message: String,
}
