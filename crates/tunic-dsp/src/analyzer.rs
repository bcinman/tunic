use std::f32::consts::TAU;
use std::sync::Arc;

use rustfft::{Fft, FftPlanner, num_complex::Complex32};

const UPDATES_PER_SECOND: f64 = 60.0;
const FFT_SIZE: usize = 4_096;
const PEAK_DECAY_DB_PER_SECOND: f32 = 20.0;
const RMS_DECAY_DB_PER_SECOND: f32 = 12.0;
const SPECTRUM_DECAY_DB_PER_SECOND: f32 = 30.0;
const SPECTRUM_ATTACK: f32 = 0.65;

pub const SPECTRUM_POINT_COUNT: usize = 256;
pub const SPECTRUM_MIN_FREQUENCY_HZ: f32 = 20.0;
pub const SPECTRUM_MAX_FREQUENCY_HZ: f32 = 20_000.0;

#[must_use]
pub fn spectrum_frequency_hz(index: usize) -> f32 {
    let ratio = index.min(SPECTRUM_POINT_COUNT - 1) as f32 / (SPECTRUM_POINT_COUNT - 1) as f32;
    SPECTRUM_MIN_FREQUENCY_HZ * (SPECTRUM_MAX_FREQUENCY_HZ / SPECTRUM_MIN_FREQUENCY_HZ).powf(ratio)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChannelLevels {
    pub peak: f32,
    pub rms: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StereoLevels {
    pub left: ChannelLevels,
    pub right: ChannelLevels,
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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnalysisFrame {
    pub levels: StereoLevels,
    pub spectrum: Spectrum,
}

pub struct Analyzer {
    level_meter: LevelMeter,
    spectrum_meter: SpectrumMeter,
}

impl Analyzer {
    #[must_use]
    pub fn new(sample_rate_hz: f64) -> Self {
        Self {
            level_meter: LevelMeter::new(sample_rate_hz),
            spectrum_meter: SpectrumMeter::new(sample_rate_hz),
        }
    }

    pub fn observe(&mut self, interleaved_stereo: &[f32]) -> Option<AnalysisFrame> {
        self.spectrum_meter.observe(interleaved_stereo);
        self.level_meter
            .observe(interleaved_stereo)
            .filter(|_| self.spectrum_meter.is_ready())
            .map(|levels| AnalysisFrame {
                levels,
                spectrum: self.spectrum_meter.current(),
            })
    }

    pub fn reset(&mut self) {
        self.level_meter.reset();
        self.spectrum_meter.reset();
    }
}

struct LevelMeter {
    window_frames: usize,
    frames: usize,
    left_peak: f32,
    right_peak: f32,
    left_square_sum: f64,
    right_square_sum: f64,
    smoothed: StereoLevels,
    peak_decay: f32,
    rms_decay: f32,
}

impl LevelMeter {
    fn new(sample_rate_hz: f64) -> Self {
        let window_frames = (sample_rate_hz / UPDATES_PER_SECOND).round() as usize;
        Self::with_window_frames_and_rate(window_frames, sample_rate_hz)
    }

    #[cfg(test)]
    fn with_window_frames(window_frames: usize) -> Self {
        Self::with_window_frames_and_rate(window_frames, window_frames as f64 * UPDATES_PER_SECOND)
    }

    fn with_window_frames_and_rate(window_frames: usize, sample_rate_hz: f64) -> Self {
        let update_rate = sample_rate_hz / window_frames.max(1) as f64;
        Self {
            window_frames: window_frames.max(1),
            frames: 0,
            left_peak: 0.0,
            right_peak: 0.0,
            left_square_sum: 0.0,
            right_square_sum: 0.0,
            smoothed: StereoLevels::default(),
            peak_decay: decay_multiplier(PEAK_DECAY_DB_PER_SECOND, update_rate),
            rms_decay: decay_multiplier(RMS_DECAY_DB_PER_SECOND, update_rate),
        }
    }

    fn observe(&mut self, interleaved_stereo: &[f32]) -> Option<StereoLevels> {
        let mut latest = None;
        for frame in interleaved_stereo.as_chunks::<2>().0 {
            let left = frame[0];
            let right = frame[1];
            self.left_peak = self.left_peak.max(left.abs());
            self.right_peak = self.right_peak.max(right.abs());
            self.left_square_sum += f64::from(left) * f64::from(left);
            self.right_square_sum += f64::from(right) * f64::from(right);
            self.frames += 1;

            if self.frames == self.window_frames {
                let frames = self.frames as f64;
                let measured = StereoLevels {
                    left: ChannelLevels {
                        peak: self.left_peak,
                        rms: (self.left_square_sum / frames).sqrt() as f32,
                    },
                    right: ChannelLevels {
                        peak: self.right_peak,
                        rms: (self.right_square_sum / frames).sqrt() as f32,
                    },
                };
                self.smoothed.left = smooth_levels(
                    self.smoothed.left,
                    measured.left,
                    self.peak_decay,
                    self.rms_decay,
                );
                self.smoothed.right = smooth_levels(
                    self.smoothed.right,
                    measured.right,
                    self.peak_decay,
                    self.rms_decay,
                );
                latest = Some(self.smoothed);
                self.frames = 0;
                self.left_peak = 0.0;
                self.right_peak = 0.0;
                self.left_square_sum = 0.0;
                self.right_square_sum = 0.0;
            }
        }
        latest
    }

    fn reset(&mut self) {
        self.frames = 0;
        self.left_peak = 0.0;
        self.right_peak = 0.0;
        self.left_square_sum = 0.0;
        self.right_square_sum = 0.0;
        self.smoothed = StereoLevels::default();
    }
}

fn smooth_levels(
    current: ChannelLevels,
    measured: ChannelLevels,
    peak_decay: f32,
    rms_decay: f32,
) -> ChannelLevels {
    ChannelLevels {
        peak: smooth_with_decay(current.peak, measured.peak, 1.0, peak_decay),
        rms: smooth_with_decay(current.rms, measured.rms, 0.35, rms_decay),
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
        Self {
            hop_frames: (sample_rate_hz / UPDATES_PER_SECOND).round().max(1.0) as usize,
            frames_since_analysis: 0,
            filled: 0,
            write_index: 0,
            left: vec![0.0; FFT_SIZE],
            right: vec![0.0; FFT_SIZE],
            window,
            fft: SpectrumFft::new(FFT_SIZE, sample_rate_hz),
            smoothed: Spectrum::default(),
            decay: decay_multiplier(SPECTRUM_DECAY_DB_PER_SECOND, UPDATES_PER_SECOND),
            ready: false,
        }
    }

    fn observe(&mut self, interleaved_stereo: &[f32]) {
        let fft_size = self.left.len();
        for frame in interleaved_stereo.as_chunks::<2>().0 {
            self.left[self.write_index] = frame[0];
            self.right[self.write_index] = frame[1];
            self.write_index = (self.write_index + 1) % fft_size;
            self.filled = (self.filled + 1).min(fft_size);
            self.frames_since_analysis += 1;

            if self.filled == fft_size && self.frames_since_analysis >= self.hop_frames {
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
    point_bins: [(usize, usize); SPECTRUM_POINT_COUNT],
}

impl SpectrumFft {
    fn new(fft_size: usize, sample_rate_hz: f64) -> Self {
        let plan = FftPlanner::new().plan_fft_forward(fft_size);
        let scratch = vec![Complex32::default(); plan.get_inplace_scratch_len()];
        Self {
            plan,
            buffer: vec![Complex32::default(); fft_size],
            scratch,
            point_bins: spectrum_point_bins(fft_size, sample_rate_hz),
        }
    }

    fn analyze(
        &mut self,
        samples: &[f32],
        start: usize,
        window: &[f32],
        points: &mut [f32; SPECTRUM_POINT_COUNT],
    ) {
        let fft_size = self.buffer.len();
        for index in 0..fft_size {
            self.buffer[index] =
                Complex32::new(samples[(start + index) % fft_size] * window[index], 0.0);
        }
        self.plan
            .process_with_scratch(&mut self.buffer, &mut self.scratch);

        let amplitude_scale = 4.0 / fft_size as f32;
        for (point, &(first, last)) in points.iter_mut().zip(&self.point_bins) {
            for value in &self.buffer[first..=last] {
                let amplitude = value.norm() * amplitude_scale;
                *point = point.max(amplitude);
            }
        }
    }
}

fn spectrum_point_bins(
    fft_size: usize,
    sample_rate_hz: f64,
) -> [(usize, usize); SPECTRUM_POINT_COUNT] {
    let bin_width_hz = sample_rate_hz / fft_size as f64;
    std::array::from_fn(|index| {
        let center = f64::from(spectrum_frequency_hz(index));
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
        let first = ((lower_hz / bin_width_hz).ceil() as usize)
            .max(1)
            .min(fft_size / 2);
        let last = ((upper_hz / bin_width_hz).floor() as usize)
            .max(nearest)
            .min(fft_size / 2);
        (first.min(last), last)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        Analyzer, ChannelLevels, FFT_SIZE, LevelMeter, SPECTRUM_POINT_COUNT, Spectrum,
        SpectrumMeter, StereoLevels, UPDATES_PER_SECOND, spectrum_frequency_hz,
    };

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
    fn analyzer_reports_levels_and_spectrum_together() {
        const SAMPLE_RATE: f32 = 48_000.0;
        const TONE_HZ: f32 = SAMPLE_RATE * 84.0 / FFT_SIZE as f32;
        let mut analyzer = Analyzer::new(f64::from(SAMPLE_RATE));
        let samples = (0..FFT_SIZE)
            .flat_map(|frame| {
                let sample = (std::f32::consts::TAU * TONE_HZ * frame as f32 / SAMPLE_RATE).sin();
                [sample, sample]
            })
            .collect::<Vec<_>>();

        let frame = analyzer.observe(&samples).unwrap();

        assert!(frame.levels.left.peak > 0.99);
        assert!(frame.spectrum.points[nearest_spectrum_point(TONE_HZ)] > 0.9);
    }

    #[test]
    fn level_meter_publishes_asymmetric_peak_and_rms_windows() {
        let mut meter = LevelMeter::with_window_frames(2);

        let levels = meter.observe(&[1.0, -0.5, 0.0, -0.5]);

        assert_eq!(
            levels,
            Some(StereoLevels {
                left: ChannelLevels {
                    peak: 1.0,
                    rms: std::f32::consts::FRAC_1_SQRT_2,
                },
                right: ChannelLevels {
                    peak: 0.5,
                    rms: 0.5,
                },
            })
        );
    }

    #[test]
    fn level_meter_decays_instead_of_dropping_to_silence() {
        let mut meter = LevelMeter::with_window_frames(2);
        let initial = meter.observe(&[1.0, 0.5, -1.0, -0.5]).unwrap();

        let decayed = meter.observe(&[0.0; 4]).unwrap();

        assert!(decayed.left.peak < initial.left.peak);
        assert!(decayed.left.peak > 0.9);
        assert!(decayed.left.rms < initial.left.rms);
        assert!(decayed.left.rms > 0.9);
    }

    #[test]
    fn spectrum_meter_separates_asymmetric_tones_and_decays() {
        const SAMPLE_RATE: f32 = 48_000.0;
        let mut meter = SpectrumMeter::new(f64::from(SAMPLE_RATE));
        let samples = (0..FFT_SIZE)
            .flat_map(|frame| {
                let time = frame as f32 / SAMPLE_RATE;
                [
                    0.5 * (std::f32::consts::TAU * 1_000.0 * time).sin(),
                    0.5 * (std::f32::consts::TAU * 8_000.0 * time).sin(),
                ]
            })
            .collect::<Vec<_>>();

        meter.observe(&samples);
        let measured = meter.current();
        let one_khz = nearest_spectrum_point(1_000.0);
        let eight_khz = nearest_spectrum_point(8_000.0);
        let four_hundred_hz = nearest_spectrum_point(400.0);

        assert!(meter.is_ready());
        assert!(measured.points[one_khz] > 0.2);
        assert!(measured.points[eight_khz] > 0.2);
        assert!(measured.points[four_hundred_hz] < 0.01);

        meter.left.fill(0.0);
        meter.right.fill(0.0);
        meter.analyze();
        let decayed = meter.current();
        assert!(decayed.points[one_khz] < measured.points[one_khz]);
        assert!(decayed.points[one_khz] > 0.0);

        meter.reset();
        assert!(!meter.is_ready());
        assert_eq!(meter.current(), Spectrum::default());
    }

    #[test]
    fn spectrum_decays_thirty_decibels_per_second() {
        const SAMPLE_RATE: f32 = 48_000.0;
        const TONE_HZ: f32 = SAMPLE_RATE * 84.0 / FFT_SIZE as f32;
        let mut meter = SpectrumMeter::new(f64::from(SAMPLE_RATE));
        let samples = (0..FFT_SIZE)
            .flat_map(|frame| {
                let sample = (std::f32::consts::TAU * TONE_HZ * frame as f32 / SAMPLE_RATE).sin();
                [sample, sample]
            })
            .collect::<Vec<_>>();
        meter.observe(&samples);
        let point = nearest_spectrum_point(TONE_HZ);
        let initial = meter.current().points[point];
        meter.left.fill(0.0);
        meter.right.fill(0.0);

        for _ in 0..UPDATES_PER_SECOND as usize {
            meter.analyze();
        }

        let decay_db = 20.0 * (meter.current().points[point] / initial).log10();
        assert!((decay_db + 30.0).abs() < 0.01, "decayed {decay_db} dB");
    }

    #[test]
    fn spectrum_points_use_valid_fft_bins_at_supported_sample_rates() {
        for sample_rate_hz in [48_000.0, 192_000.0] {
            let meter = SpectrumMeter::new(sample_rate_hz);
            for &(first, last) in &meter.fft.point_bins {
                assert!(first >= 1);
                assert!(first <= last);
                assert!(last <= FFT_SIZE / 2);
            }
        }
    }

    #[test]
    fn spectrum_assigns_tones_between_rounded_centers_to_a_band() {
        const SAMPLE_RATE: f32 = 48_000.0;
        const TONE_HZ: f32 = 14_144.531;
        let mut meter = SpectrumMeter::new(f64::from(SAMPLE_RATE));
        let samples = (0..FFT_SIZE)
            .flat_map(|frame| {
                let sample =
                    0.5 * (std::f32::consts::TAU * TONE_HZ * frame as f32 / SAMPLE_RATE).sin();
                [sample, sample]
            })
            .collect::<Vec<_>>();

        meter.observe(&samples);

        assert!(meter.current().points[nearest_spectrum_point(TONE_HZ)] > 0.4);
    }

    #[test]
    fn spectrum_onset_response_is_measured_in_sample_frames() {
        const SAMPLE_RATE: f32 = 48_000.0;
        const TONE_HZ: f32 = SAMPLE_RATE * 84.0 / FFT_SIZE as f32;
        const CALLBACK_FRAMES: usize = 512;
        const THRESHOLDS_DBFS: [f32; 4] = [-60.0, -36.0, -24.0, -6.0];
        let mut meter = SpectrumMeter::new(f64::from(SAMPLE_RATE));
        let point = nearest_spectrum_point(TONE_HZ);
        let silence = vec![0.0; FFT_SIZE * 2];
        meter.observe(&silence);
        let mut crossings = [None; THRESHOLDS_DBFS.len()];
        let mut elapsed_frames = 0;

        while elapsed_frames < SAMPLE_RATE as usize {
            let samples = (elapsed_frames..elapsed_frames + CALLBACK_FRAMES)
                .flat_map(|frame| {
                    let sample =
                        (std::f32::consts::TAU * TONE_HZ * frame as f32 / SAMPLE_RATE).sin();
                    [sample, sample]
                })
                .collect::<Vec<_>>();
            meter.observe(&samples);
            elapsed_frames += CALLBACK_FRAMES;
            let amplitude = meter.current().points[point];
            for (crossing, threshold_dbfs) in crossings.iter_mut().zip(THRESHOLDS_DBFS) {
                let threshold = 10.0_f32.powf(threshold_dbfs / 20.0);
                if crossing.is_none() && amplitude >= threshold {
                    *crossing = Some(elapsed_frames);
                }
            }
        }

        for crossing in crossings {
            assert!(crossing.is_some());
        }
        assert!(crossings[0].unwrap() <= 2 * meter.hop_frames);
        assert!(crossings[3].unwrap() <= FFT_SIZE + 2 * meter.hop_frames);
    }
}
