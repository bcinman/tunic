use std::time::Duration;

use gpui::{
    Bounds, ContentMask, Context, IntoElement, PathBuilder, Render, Task, Window, canvas, point,
    prelude::*, px, rgba, size,
};
use tunic_engine::{SPECTRUM_POINT_COUNT, Spectrum, TelemetryReader};

const SPECTRUM_FLOOR_DB: f32 = -72.0;
const SPECTRUM_FADE_BANDS: usize = 96;
const SPECTRUM_FILL_MAX_ALPHA: u8 = 56;
const SPECTRUM_CURVE_MAX_ALPHA: u8 = 120;

pub(super) struct SpectrumView {
    telemetry: TelemetryReader,
    spectrum: Spectrum,
    _refresh_task: Task<()>,
}

impl SpectrumView {
    pub(super) fn new(telemetry: TelemetryReader, cx: &mut Context<Self>) -> Self {
        let refresh_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_micros(16_667))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        let Some(frame) = this.telemetry.try_latest() else {
                            return;
                        };
                        if frame.spectrum != this.spectrum {
                            this.spectrum = frame.spectrum;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            telemetry,
            spectrum: Spectrum::default(),
            _refresh_task: refresh_task,
        }
    }
}

impl Render for SpectrumView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let spectrum = self.spectrum;
        canvas(
            move |bounds, _, _| {
                let mut fill = PathBuilder::fill();
                fill.move_to(point(bounds.left(), bounds.bottom()));
                for (index, amplitude) in spectrum.points.into_iter().enumerate() {
                    let x = bounds.left() + bounds.size.width * spectrum_point_ratio(index);
                    let y =
                        bounds.top() + bounds.size.height * spectrum_amplitude_to_ratio(amplitude);
                    fill.line_to(point(x, y));
                }
                fill.line_to(point(bounds.right(), bounds.bottom()));
                fill.close();

                let mut curve = PathBuilder::stroke(px(1.0));
                for (index, amplitude) in spectrum.points.into_iter().enumerate() {
                    let x = bounds.left() + bounds.size.width * spectrum_point_ratio(index);
                    let y =
                        bounds.top() + bounds.size.height * spectrum_amplitude_to_ratio(amplitude);
                    if index == 0 {
                        curve.move_to(point(x, y));
                    } else {
                        curve.line_to(point(x, y));
                    }
                }
                (fill.build().ok(), curve.build().ok())
            },
            |bounds, (fill, curve), window, _| {
                for band in 0..SPECTRUM_FADE_BANDS {
                    let top_ratio = band as f32 / SPECTRUM_FADE_BANDS as f32;
                    let bottom_ratio = (band + 1) as f32 / SPECTRUM_FADE_BANDS as f32;
                    let vertical_ratio = (top_ratio + bottom_ratio) / 2.0;
                    let fill_alpha = spectrum_fade_alpha(vertical_ratio, SPECTRUM_FILL_MAX_ALPHA);
                    let curve_alpha = spectrum_fade_alpha(vertical_ratio, SPECTRUM_CURVE_MAX_ALPHA);
                    if fill_alpha == 0 && curve_alpha == 0 {
                        continue;
                    }

                    let top = bounds.top() + bounds.size.height * top_ratio;
                    let height = bounds.size.height * (bottom_ratio - top_ratio);
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: Bounds {
                                origin: point(bounds.left(), top),
                                size: size(bounds.size.width, height),
                            },
                        }),
                        |window| {
                            if let Some(fill) = fill.as_ref()
                                && fill_alpha > 0
                            {
                                window.paint_path(
                                    fill.clone(),
                                    rgba(0x297ca600 | u32::from(fill_alpha)),
                                );
                            }
                            if let Some(curve) = curve.as_ref()
                                && curve_alpha > 0
                            {
                                window.paint_path(
                                    curve.clone(),
                                    rgba(0x5cc8ff00 | u32::from(curve_alpha)),
                                );
                            }
                        },
                    );
                }
            },
        )
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .size_full()
    }
}

fn spectrum_amplitude_to_ratio(amplitude: f32) -> f32 {
    let decibels = if amplitude > 0.0 {
        20.0 * amplitude.log10()
    } else {
        SPECTRUM_FLOOR_DB
    };
    -decibels.clamp(SPECTRUM_FLOOR_DB, 0.0) / -SPECTRUM_FLOOR_DB
}

fn spectrum_point_ratio(index: usize) -> f32 {
    index as f32 / (SPECTRUM_POINT_COUNT - 1) as f32
}

fn spectrum_fade_alpha(vertical_ratio: f32, max_alpha: u8) -> u8 {
    let ratio = vertical_ratio.clamp(0.0, 1.0);
    (f32::from(max_alpha) * (1.0 - ratio.powi(4))).round() as u8
}

#[cfg(test)]
mod tests {
    use super::{
        SPECTRUM_CURVE_MAX_ALPHA, SPECTRUM_FILL_MAX_ALPHA, spectrum_amplitude_to_ratio,
        spectrum_fade_alpha,
    };

    #[test]
    fn projection_maps_dbfs_to_the_graph_height() {
        assert_eq!(spectrum_amplitude_to_ratio(1.0), 0.0);
        assert!((spectrum_amplitude_to_ratio(0.001) - 5.0 / 6.0).abs() < 1e-6);
        assert_eq!(spectrum_amplitude_to_ratio(0.0), 1.0);
        assert_eq!(spectrum_amplitude_to_ratio(2.0), 0.0);
    }

    #[test]
    fn spectrum_uses_a_smooth_non_linear_fade() {
        assert_eq!(spectrum_fade_alpha(0.0, SPECTRUM_FILL_MAX_ALPHA), 56);
        assert_eq!(spectrum_fade_alpha(1.0, SPECTRUM_FILL_MAX_ALPHA), 0);
        assert_eq!(spectrum_fade_alpha(0.25, SPECTRUM_FILL_MAX_ALPHA), 56);
        assert_eq!(spectrum_fade_alpha(0.75, SPECTRUM_FILL_MAX_ALPHA), 38);
        assert_eq!(spectrum_fade_alpha(0.75, SPECTRUM_CURVE_MAX_ALPHA), 82);
    }
}
