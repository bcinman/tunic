//! Callback-local post-processor level and spectrum analysis.
//!
//! Construction allocates FFT and history storage. Observation reuses it so
//! subscribed analysis remains allocation-free in the audio callback.

use std::f32::consts::TAU;
use std::sync::Arc;

use rustfft::{Fft, FftPlanner, num_complex::Complex32};

use super::{
    SampleRateHz,
    telemetry::{
        ChannelLevels, SPECTRUM_POINT_COUNT, Spectrum, StereoLevels, TelemetryFrame,
        spectrum_frequency_hz,
    },
};

const UPDATES_PER_SECOND: f64 = 60.0;
const FFT_SIZE: usize = 4_096;
const SPECTRUM_DECAY_DB_PER_SECOND: f32 = 30.0;
const SPECTRUM_ATTACK: f32 = 0.65;

pub(crate) struct Analyzer {
    levels: LevelMeter,
    spectrum: SpectrumMeter,
}

impl Analyzer {
    pub(crate) fn new(sample_rate: SampleRateHz) -> Self {
        let sample_rate_hz = sample_rate.into_inner();
        Self {
            levels: LevelMeter::new(sample_rate_hz),
            spectrum: SpectrumMeter::new(sample_rate_hz),
        }
    }

    pub(crate) fn observe(&mut self, frames: &[[f32; 2]]) -> Option<TelemetryFrame> {
        self.spectrum.observe(frames);
        self.levels
            .observe(frames)
            .filter(|_| self.spectrum.is_ready())
            .map(|levels| TelemetryFrame {
                sequence: 0,
                levels,
                spectrum: self.spectrum.current(),
            })
    }

    pub(crate) fn reset(&mut self) {
        self.levels.reset();
        self.spectrum.reset();
    }
}

struct LevelMeter {
    window_frames: usize,
    frames: usize,
    left_peak: f32,
    right_peak: f32,
    left_square_sum: f64,
    right_square_sum: f64,
}

impl LevelMeter {
    fn new(sample_rate_hz: f64) -> Self {
        let window_frames = (sample_rate_hz / UPDATES_PER_SECOND).round() as usize;
        Self::with_window_frames(window_frames)
    }

    fn with_window_frames(window_frames: usize) -> Self {
        Self {
            window_frames: window_frames.max(1),
            frames: 0,
            left_peak: 0.0,
            right_peak: 0.0,
            left_square_sum: 0.0,
            right_square_sum: 0.0,
        }
    }

    fn observe(&mut self, frames: &[[f32; 2]]) -> Option<StereoLevels> {
        let mut latest = None;
        for &[left, right] in frames {
            self.left_peak = self.left_peak.max(left.abs());
            self.right_peak = self.right_peak.max(right.abs());
            self.left_square_sum += f64::from(left) * f64::from(left);
            self.right_square_sum += f64::from(right) * f64::from(right);
            self.frames += 1;

            if self.frames == self.window_frames {
                let frame_count = self.frames as f64;
                let measured = StereoLevels {
                    left: ChannelLevels {
                        peak: self.left_peak,
                        rms: (self.left_square_sum / frame_count).sqrt() as f32,
                    },
                    right: ChannelLevels {
                        peak: self.right_peak,
                        rms: (self.right_square_sum / frame_count).sqrt() as f32,
                    },
                };
                latest = Some(measured);
                self.clear_window();
            }
        }
        latest
    }

    fn reset(&mut self) {
        self.clear_window();
    }

    fn clear_window(&mut self) {
        self.frames = 0;
        self.left_peak = 0.0;
        self.right_peak = 0.0;
        self.left_square_sum = 0.0;
        self.right_square_sum = 0.0;
    }
}

fn smooth_with_decay(current: f32, measured: f32, attack: f32, decay: f32) -> f32 {
    if current == 0.0 {
        measured
    } else if measured >= current {
        current + (measured - current) * attack
    } else {
        measured.max(current * decay)
    }
}

fn decay_multiplier(decibels_per_second: f32, updates_per_second: f64) -> f32 {
    10.0_f32.powf(-decibels_per_second / (20.0 * updates_per_second as f32))
}

struct SpectrumMeter {
    hop_frames: usize,
    frames_since_analysis: usize,
    filled: usize,
    write_index: usize,
    left: Vec<f32>,
    right: Vec<f32>,
    window: Vec<f32>,
    fft: SpectrumFft,
    smoothed: Spectrum,
    decay: f32,
    ready: bool,
}

impl SpectrumMeter {
    fn new(sample_rate_hz: f64) -> Self {
        let mut window = vec![0.0; FFT_SIZE];
        for (index, value) in window.iter_mut().enumerate() {
            *value = 0.5 - 0.5 * (TAU * index as f32 / (FFT_SIZE - 1) as f32).cos();
        }
        let window_sum = window.iter().sum();
        Self {
            hop_frames: (sample_rate_hz / UPDATES_PER_SECOND).round().max(1.0) as usize,
            frames_since_analysis: 0,
            filled: 0,
            write_index: 0,
            left: vec![0.0; FFT_SIZE],
            right: vec![0.0; FFT_SIZE],
            window,
            fft: SpectrumFft::new(sample_rate_hz, window_sum),
            smoothed: Spectrum::default(),
            decay: decay_multiplier(SPECTRUM_DECAY_DB_PER_SECOND, UPDATES_PER_SECOND),
            ready: false,
        }
    }

    fn observe(&mut self, frames: &[[f32; 2]]) {
        for &[left, right] in frames {
            self.left[self.write_index] = left;
            self.right[self.write_index] = right;
            self.write_index = (self.write_index + 1) % FFT_SIZE;
            self.filled = (self.filled + 1).min(FFT_SIZE);
            self.frames_since_analysis += 1;

            if self.filled == FFT_SIZE && self.frames_since_analysis >= self.hop_frames {
                self.analyze();
                self.frames_since_analysis = 0;
            }
        }
    }

    fn current(&self) -> Spectrum {
        self.smoothed
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    fn reset(&mut self) {
        self.frames_since_analysis = 0;
        self.filled = 0;
        self.write_index = 0;
        self.smoothed = Spectrum::default();
        self.ready = false;
    }

    fn analyze(&mut self) {
        let mut measured = [0.0; SPECTRUM_POINT_COUNT];
        self.fft
            .analyze(&self.left, self.write_index, &self.window, &mut measured);
        self.fft
            .analyze(&self.right, self.write_index, &self.window, &mut measured);
        for (smoothed, measured) in self.smoothed.points.iter_mut().zip(measured) {
            *smoothed = smooth_with_decay(*smoothed, measured, SPECTRUM_ATTACK, self.decay);
        }
        self.ready = true;
    }
}

struct SpectrumFft {
    plan: Arc<dyn Fft<f32>>,
    buffer: Vec<Complex32>,
    scratch: Vec<Complex32>,
    point_bins: [Option<(usize, usize)>; SPECTRUM_POINT_COUNT],
    interior_amplitude_scale: f32,
    nyquist_amplitude_scale: f32,
}

impl SpectrumFft {
    fn new(sample_rate_hz: f64, window_sum: f32) -> Self {
        let plan = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let scratch = vec![Complex32::default(); plan.get_inplace_scratch_len()];
        Self {
            plan,
            buffer: vec![Complex32::default(); FFT_SIZE],
            scratch,
            point_bins: spectrum_point_bins(sample_rate_hz),
            interior_amplitude_scale: 2.0 / window_sum,
            nyquist_amplitude_scale: 1.0 / window_sum,
        }
    }

    fn analyze(
        &mut self,
        samples: &[f32],
        start: usize,
        window: &[f32],
        points: &mut [f32; SPECTRUM_POINT_COUNT],
    ) {
        for index in 0..FFT_SIZE {
            self.buffer[index] =
                Complex32::new(samples[(start + index) % FFT_SIZE] * window[index], 0.0);
        }
        self.plan
            .process_with_scratch(&mut self.buffer, &mut self.scratch);

        for (point, bins) in points.iter_mut().zip(&self.point_bins) {
            let Some((first, last)) = *bins else {
                continue;
            };
            for (bin, value) in self.buffer[first..=last].iter().enumerate() {
                let bin = first + bin;
                let scale = if bin == FFT_SIZE / 2 {
                    self.nyquist_amplitude_scale
                } else {
                    self.interior_amplitude_scale
                };
                *point = point.max(value.norm() * scale);
            }
        }
    }
}

fn spectrum_point_bins(sample_rate_hz: f64) -> [Option<(usize, usize)>; SPECTRUM_POINT_COUNT] {
    let bin_width_hz = sample_rate_hz / FFT_SIZE as f64;
    let nyquist_hz = sample_rate_hz / 2.0;
    std::array::from_fn(|index| {
        let center = f64::from(spectrum_frequency_hz(index));
        if center > nyquist_hz {
            return None;
        }
        let lower_hz = if index == 0 {
            center
        } else {
            let previous = f64::from(spectrum_frequency_hz(index - 1));
            (previous * center).sqrt()
        };
        let upper_hz = if index + 1 == SPECTRUM_POINT_COUNT {
            center
        } else {
            let next = f64::from(spectrum_frequency_hz(index + 1));
            (center * next).sqrt()
        };
        let nearest = ((center / bin_width_hz).round() as usize).max(1);
        let first = ((lower_hz / bin_width_hz).ceil() as usize).clamp(1, FFT_SIZE / 2);
        let last = ((upper_hz / bin_width_hz).floor() as usize)
            .max(nearest)
            .min(FFT_SIZE / 2);
        Some((first.min(last), last))
    })
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::{Analyzer, FFT_SIZE, LevelMeter};
    use crate::{SPECTRUM_POINT_COUNT, SampleRateHz, spectrum_frequency_hz};

    fn nearest_spectrum_point(frequency_hz: f32) -> usize {
        (0..SPECTRUM_POINT_COUNT)
            .min_by(|&left, &right| {
                (spectrum_frequency_hz(left) - frequency_hz)
                    .abs()
                    .total_cmp(&(spectrum_frequency_hz(right) - frequency_hz).abs())
            })
            .unwrap()
    }

    #[test]
    fn level_meter_reports_asymmetric_peak_and_rms() {
        let mut meter = LevelMeter::with_window_frames(2);

        let levels = meter.observe(&[[1.0, -0.5], [0.0, -0.5]]).unwrap();

        assert_eq!(levels.left.peak, 1.0);
        assert_eq!(levels.left.rms, std::f32::consts::FRAC_1_SQRT_2);
        assert_eq!(levels.right.peak, 0.5);
        assert_eq!(levels.right.rms, 0.5);
    }

    #[test]
    fn level_meter_reports_each_window_without_display_ballistics() {
        let mut meter = LevelMeter::with_window_frames(2);

        assert!(meter.observe(&[[1.0, 1.0]; 2]).is_some());
        let levels = meter.observe(&[[0.25, 0.5]; 2]).unwrap();

        assert_eq!(levels.left.peak, 0.25);
        assert_eq!(levels.left.rms, 0.25);
        assert_eq!(levels.right.peak, 0.5);
        assert_eq!(levels.right.rms, 0.5);
    }

    #[test]
    fn analyzer_reports_a_known_tone() {
        const SAMPLE_RATE: f32 = 48_000.0;
        const TONE_HZ: f32 = SAMPLE_RATE * 84.0 / FFT_SIZE as f32;
        let mut analyzer = Analyzer::new(SampleRateHz::try_new(f64::from(SAMPLE_RATE)).unwrap());
        let frames = (0..FFT_SIZE)
            .map(|frame| {
                let sample = (TAU * TONE_HZ * frame as f32 / SAMPLE_RATE).sin();
                [sample, sample]
            })
            .collect::<Vec<_>>();

        let frame = analyzer.observe(&frames).unwrap();

        assert!(frame.levels.left.peak > 0.99);
        let magnitude = frame.spectrum.points[nearest_spectrum_point(TONE_HZ)];
        assert!((magnitude - 1.0).abs() < 0.001, "magnitude {magnitude}");
    }

    #[test]
    fn analyzer_does_not_double_nyquist_amplitude() {
        const SAMPLE_RATE: f32 = 40_000.0;
        const AMPLITUDE: f32 = 0.5;
        let mut analyzer = Analyzer::new(SampleRateHz::try_new(f64::from(SAMPLE_RATE)).unwrap());
        let frames = (0..FFT_SIZE)
            .map(|frame| {
                let sample = if frame.is_multiple_of(2) {
                    AMPLITUDE
                } else {
                    -AMPLITUDE
                };
                [sample, sample]
            })
            .collect::<Vec<_>>();

        let frame = analyzer.observe(&frames).unwrap();
        let magnitude = frame.spectrum.points[SPECTRUM_POINT_COUNT - 1];

        assert!(
            (magnitude - AMPLITUDE).abs() < 0.001,
            "magnitude {magnitude}"
        );
    }

    #[test]
    fn points_above_nyquist_are_zero() {
        const SAMPLE_RATE: f32 = 32_000.0;
        let mut analyzer = Analyzer::new(SampleRateHz::try_new(f64::from(SAMPLE_RATE)).unwrap());
        let frame = analyzer.observe(&[[1.0, 1.0]; FFT_SIZE]).unwrap();

        for (index, magnitude) in frame.spectrum.points.iter().enumerate() {
            if spectrum_frequency_hz(index) > SAMPLE_RATE / 2.0 {
                assert_eq!(*magnitude, 0.0);
            }
        }
    }
}
