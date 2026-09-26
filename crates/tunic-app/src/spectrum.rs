use std::time::Duration;

use gpui::{
    Context, IntoElement, PathBuilder, Render, Task, Window, canvas, point, prelude::*, px, rgba,
};
use tunic_engine::{SPECTRUM_POINT_COUNT, Spectrum, TelemetryReader};

const SPECTRUM_FLOOR_DB: f32 = -72.0;

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
            |_, (fill, curve), window, _| {
                if let Some(fill) = fill {
                    window.paint_path(fill, rgba(0x297ca638));
                }
                if let Some(curve) = curve {
                    window.paint_path(curve, rgba(0x5cc8ff78));
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

#[cfg(test)]
mod tests {
    use super::spectrum_amplitude_to_ratio;

    #[test]
    fn projection_maps_dbfs_to_the_graph_height() {
        assert_eq!(spectrum_amplitude_to_ratio(1.0), 0.0);
        assert!((spectrum_amplitude_to_ratio(0.001) - 5.0 / 6.0).abs() < 1e-6);
        assert_eq!(spectrum_amplitude_to_ratio(0.0), 1.0);
        assert_eq!(spectrum_amplitude_to_ratio(2.0), 0.0);
    }
}
