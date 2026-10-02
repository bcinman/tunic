use gpui::{
    Context, IntoElement, PathBuilder, Render, Window, canvas, div, point, prelude::*, rgba,
};
use tunic_core::{Controller, SPECTRUM_POINT_COUNT, Spectrum, Telemetry};

const SPECTRUM_FLOOR_DB: f32 = -90.0;
const SPECTRUM_DECAY_DB_PER_SECOND: f32 = 40.0;
const TELEMETRY_UPDATES_PER_SECOND: f32 = 60.0;

pub(crate) struct SpectrumView {
    telemetry: Option<Telemetry>,
    spectrum: Spectrum,
}

impl SpectrumView {
    pub(crate) fn new(controller: Option<&Controller>) -> Self {
        Self {
            telemetry: controller.map(Controller::subscribe_telemetry),
            spectrum: Spectrum::default(),
        }
    }

    pub(crate) fn replace_controller(&mut self, controller: Option<&Controller>) {
        self.telemetry = controller.map(Controller::subscribe_telemetry);
        self.spectrum = Spectrum::default();
    }

    fn update(&mut self) {
        if let Some(telemetry) = &self.telemetry {
            telemetry.for_each_unseen(|frame| {
                animate_spectrum(
                    &mut self.spectrum,
                    &frame.spectrum,
                    1.0 / TELEMETRY_UPDATES_PER_SECOND,
                );
            });
        }
    }
}

impl Render for SpectrumView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.update();
        window.request_animation_frame();
        let points = self.spectrum.points.map(spectrum_fraction);

        div().size_full().child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let mut path = PathBuilder::fill();
                    path.move_to(bounds.bottom_left());
                    for (index, fraction) in points.iter().copied().enumerate() {
                        let x = bounds.origin.x
                            + bounds.size.width
                                * (index as f32 / (SPECTRUM_POINT_COUNT - 1) as f32);
                        let y = bounds.origin.y + bounds.size.height * (1.0 - fraction);
                        path.line_to(point(x, y));
                    }
                    path.line_to(bounds.bottom_right());
                    path.line_to(bounds.bottom_left());
                    if let Ok(path) = path.build() {
                        window.paint_path(path, rgba(0xb8bcc01a));
                    }
                },
            )
            .size_full(),
        )
    }
}

fn animate_spectrum(current: &mut Spectrum, measured: &Spectrum, elapsed_seconds: f32) {
    for (current, measured) in current.points.iter_mut().zip(measured.points) {
        *current = animate_spectrum_point(*current, measured, elapsed_seconds);
    }
}

pub(crate) fn animate_spectrum_point(current: f32, measured: f32, elapsed_seconds: f32) -> f32 {
    let current = finite_amplitude(current);
    let measured = finite_amplitude(measured);
    if current == 0.0 || measured >= current {
        measured
    } else {
        let decay = 10.0_f32.powf(-SPECTRUM_DECAY_DB_PER_SECOND * elapsed_seconds / 20.0);
        measured.max(current * decay)
    }
}

fn finite_amplitude(amplitude: f32) -> f32 {
    if amplitude.is_finite() {
        amplitude.max(0.0)
    } else {
        0.0
    }
}

pub(crate) fn spectrum_fraction(amplitude: f32) -> f32 {
    if !amplitude.is_finite() || amplitude <= 0.0 {
        return 0.0;
    }
    ((20.0 * amplitude.log10() - SPECTRUM_FLOOR_DB) / -SPECTRUM_FLOOR_DB).clamp(0.0, 1.0)
}
