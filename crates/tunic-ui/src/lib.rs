//! Shared GPUI presentation for Tunic desktop applications.

use std::{collections::VecDeque, time::Instant};

use gpui::{
    Context, Div, Entity, IntoElement, Render, Stateful, Window, canvas, div, fill, point,
    prelude::*, relative, rgb, size,
};
use tunic_core::{
    Backend, Chain, Command, Controller, DeviceId, MemoryStore, PresetCatalog, PresetQuery,
    PresetSummary, ProfileId, ProfileName, ProfileSource, SPECTRUM_POINT_COUNT, Spectrum,
    StereoLevels, Telemetry,
};
use tunic_presets::BundledCatalog;

const SYSTEM_OUTPUT: &str = "system-output";
const FLAT_PROFILE: &str = "flat";
const METER_FLOOR_DB: f32 = -60.0;
const METER_DECAY_DB_PER_SECOND: f32 = 24.0;
const SPECTRUM_FLOOR_DB: f32 = -90.0;
const SPECTRUM_DECAY_DB_PER_SECOND: f32 = 40.0;
const TELEMETRY_UPDATES_PER_SECOND: f32 = 60.0;
const FRAME_HISTORY_LENGTH: usize = 90;
const SLOW_FRAME_MILLISECONDS: f32 = 33.0;

pub struct TunicView {
    model: Model,
    telemetry: Entity<TelemetryView>,
}

struct TelemetryView {
    telemetry: Option<Telemetry>,
    levels: StereoLevels,
    spectrum: Spectrum,
    last_frame_at: Option<Instant>,
    frame_times: VecDeque<f32>,
}

impl TunicView {
    pub fn new(
        device_name: String,
        controller: Option<Controller>,
        audio_error: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let telemetry = controller.as_ref().map(Controller::subscribe_telemetry);
        Self {
            model: Model::new(device_name, controller, audio_error),
            telemetry: cx.new(|_| TelemetryView::new(telemetry)),
        }
    }

    #[must_use]
    pub fn active_chain(&self) -> Chain {
        self.model.active_chain()
    }

    pub fn replace_audio(
        &mut self,
        device_name: String,
        controller: Option<Controller>,
        audio_error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let telemetry = controller.as_ref().map(Controller::subscribe_telemetry);
        self.telemetry.update(cx, |view, cx| {
            view.telemetry = telemetry;
            view.levels = StereoLevels::default();
            view.spectrum = Spectrum::default();
            cx.notify();
        });
        self.model.device_name = device_name;
        self.model.controller = controller;
        self.model.audio_error = audio_error;
    }
}

impl TelemetryView {
    fn new(telemetry: Option<Telemetry>) -> Self {
        Self {
            telemetry,
            levels: StereoLevels::default(),
            spectrum: Spectrum::default(),
            last_frame_at: None,
            frame_times: VecDeque::with_capacity(FRAME_HISTORY_LENGTH),
        }
    }

    fn update(&mut self, elapsed_seconds: f32) {
        let mut left_peak = None;
        let mut right_peak = None;
        if let Some(telemetry) = &self.telemetry {
            telemetry.for_each_unseen(|frame| {
                left_peak = highest_peak(left_peak, frame.levels.left.peak);
                right_peak = highest_peak(right_peak, frame.levels.right.peak);
                animate_spectrum(
                    &mut self.spectrum,
                    &frame.spectrum,
                    1.0 / TELEMETRY_UPDATES_PER_SECOND,
                );
            });
        }
        self.levels.left.peak = animate_meter(self.levels.left.peak, left_peak, elapsed_seconds);
        self.levels.right.peak = animate_meter(self.levels.right.peak, right_peak, elapsed_seconds);
    }

    fn record_frame(&mut self, now: Instant) -> f32 {
        let Some(previous) = self.last_frame_at.replace(now) else {
            return 0.0;
        };
        let elapsed = now.duration_since(previous).as_secs_f32();
        if elapsed <= 0.25 {
            if self.frame_times.len() == FRAME_HISTORY_LENGTH {
                self.frame_times.pop_front();
            }
            self.frame_times.push_back(elapsed * 1_000.0);
        }
        elapsed
    }
}

impl Render for TelemetryView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let elapsed_seconds = self.record_frame(Instant::now());
        self.update(elapsed_seconds);
        window.request_animation_frame();

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(meter("L", self.levels.left.peak))
                    .child(meter("R", self.levels.right.peak)),
            )
            .child(spectrum_graph(&self.spectrum))
            .child(frame_graph(&self.frame_times))
    }
}

impl Render for TunicView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.model.selected_profile_name();
        let presets = self.model.presets.clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_6()
            .bg(rgb(0x1f2023))
            .text_color(rgb(0xf2f2f2))
            .child(div().text_xl().child("Tunic"))
            .child(format!("Device: {}", self.model.device_name))
            .child(format!("Selected profile: {selected}"))
            .child(self.telemetry.clone())
            .child(
                button("flat", "Use Flat").on_click(cx.listener(|this, _, _, cx| {
                    this.model.select_flat();
                    cx.notify();
                })),
            )
            .children(presets.into_iter().enumerate().map(|(index, preset)| {
                let label = format!("Use {} {} — {}", preset.brand, preset.model, preset.target);
                button(("preset", index), label).on_click(cx.listener(move |this, _, _, cx| {
                    this.model.select_preset(index);
                    cx.notify();
                }))
            }))
            .child(
                button("clear", "Clear selection").on_click(cx.listener(|this, _, _, cx| {
                    this.model.clear_selection();
                    cx.notify();
                })),
            )
            .when_some(self.model.audio_error.as_ref(), |view, error| {
                view.child(
                    div()
                        .text_color(rgb(0xff8a8a))
                        .child(format!("Audio unavailable: {error}")),
                )
            })
            .when_some(self.model.action_error.as_ref(), |view, error| {
                view.child(
                    div()
                        .text_color(rgb(0xff8a8a))
                        .child(format!("Error: {error}")),
                )
            })
    }
}

fn meter(label: &'static str, amplitude: f32) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().w_4().child(label))
        .child(
            div().h_2().flex_1().bg(rgb(0x35373c)).child(
                div()
                    .h_full()
                    .w(relative(meter_fraction(amplitude)))
                    .bg(rgb(0x69b578)),
            ),
        )
}

fn meter_fraction(amplitude: f32) -> f32 {
    if !amplitude.is_finite() || amplitude <= 0.0 {
        return 0.0;
    }
    ((20.0 * amplitude.log10() - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

fn animate_meter(current: f32, measured: Option<f32>, elapsed_seconds: f32) -> f32 {
    let current = if current.is_finite() {
        current.max(0.0)
    } else {
        0.0
    };
    let decay = 10.0_f32.powf(-METER_DECAY_DB_PER_SECOND * elapsed_seconds / 20.0);
    let decayed = current * decay;
    measured
        .filter(|level| level.is_finite())
        .map_or(decayed, |level| decayed.max(level.max(0.0)))
}

fn highest_peak(current: Option<f32>, measured: f32) -> Option<f32> {
    measured
        .is_finite()
        .then(|| current.map_or(measured, |current| current.max(measured)))
        .or(current)
}

fn animate_spectrum(current: &mut Spectrum, measured: &Spectrum, elapsed_seconds: f32) {
    for (current, measured) in current.points.iter_mut().zip(measured.points) {
        *current = animate_spectrum_point(*current, measured, elapsed_seconds);
    }
}

fn animate_spectrum_point(current: f32, measured: f32, elapsed_seconds: f32) -> f32 {
    let current = finite_amplitude(current);
    let measured = finite_amplitude(measured);
    if current == 0.0 {
        return measured;
    }
    if measured >= current {
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

fn spectrum_fraction(amplitude: f32) -> f32 {
    if !amplitude.is_finite() || amplitude <= 0.0 {
        return 0.0;
    }
    ((20.0 * amplitude.log10() - SPECTRUM_FLOOR_DB) / -SPECTRUM_FLOOR_DB).clamp(0.0, 1.0)
}

fn spectrum_graph(spectrum: &Spectrum) -> Div {
    let bars = spectrum.points.map(spectrum_fraction);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child("Spectrum")
        .child(
            div().h_32().bg(rgb(0x35373c)).child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let bar_width = bounds.size.width / SPECTRUM_POINT_COUNT as f32;
                        for (index, fraction) in bars.iter().copied().enumerate() {
                            let height = bounds.size.height * fraction;
                            window.paint_quad(fill(
                                gpui::Bounds {
                                    origin: point(
                                        bounds.origin.x + bar_width * index as f32,
                                        bounds.origin.y + bounds.size.height - height,
                                    ),
                                    size: size(bar_width, height),
                                },
                                rgb(0x69b578),
                            ));
                        }
                    },
                )
                .size_full(),
            ),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .text_sm()
                .child("20 Hz")
                .child("20 kHz"),
        )
}

fn frame_graph(frame_times: &VecDeque<f32>) -> Div {
    let average = if frame_times.is_empty() {
        0.0
    } else {
        frame_times.iter().sum::<f32>() / frame_times.len() as f32
    };
    let fps = if average > 0.0 {
        1_000.0 / average
    } else {
        0.0
    };
    let bars = frame_times.iter().copied().collect::<Vec<_>>();

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(format!("Frame rate: {fps:.0} fps / {average:.1} ms"))
        .child(
            div().flex().items_end().h_10().bg(rgb(0x35373c)).child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let count = bars.len() as f32;
                        if count == 0.0 {
                            return;
                        }
                        let bar_width = bounds.size.width / count;
                        for (index, milliseconds) in bars.iter().copied().enumerate() {
                            let fraction =
                                (milliseconds / SLOW_FRAME_MILLISECONDS).clamp(0.05, 1.0);
                            let height = bounds.size.height * fraction;
                            let color = if milliseconds > SLOW_FRAME_MILLISECONDS {
                                rgb(0xd16d6d)
                            } else if milliseconds > 20.0 {
                                rgb(0xd1b56d)
                            } else {
                                rgb(0x69b578)
                            };
                            window.paint_quad(fill(
                                gpui::Bounds {
                                    origin: point(
                                        bounds.origin.x + bar_width * index as f32,
                                        bounds.origin.y + bounds.size.height - height,
                                    ),
                                    size: size(bar_width, height),
                                },
                                color,
                            ));
                        }
                    },
                )
                .size_full(),
            ),
        )
}

fn button(id: impl Into<gpui::ElementId>, label: impl Into<gpui::SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_2()
        .bg(rgb(0x35373c))
        .hover(|style| style.bg(rgb(0x484b52)))
        .cursor_pointer()
        .child(label.into())
}

struct Model {
    backend: Backend,
    device: DeviceId,
    device_name: String,
    controller: Option<Controller>,
    presets: Vec<PresetSummary>,
    audio_error: Option<String>,
    action_error: Option<String>,
}

impl Model {
    fn new(
        device_name: String,
        controller: Option<Controller>,
        audio_error: Option<String>,
    ) -> Self {
        let catalog = BundledCatalog;
        let presets = catalog.list(&PresetQuery::default());
        let backend = Backend::new(MemoryStore::default(), catalog)
            .expect("the in-memory store starts with valid state");
        Self {
            backend,
            device: DeviceId::try_new(SYSTEM_OUTPUT).expect("static device ID is valid"),
            device_name,
            controller,
            presets,
            audio_error,
            action_error: None,
        }
    }

    fn selected_profile_name(&self) -> String {
        self.backend
            .state()
            .selected_profile(&self.device)
            .map_or_else(|| "None".into(), |profile| profile.name.to_string())
    }

    fn select_flat(&mut self) {
        self.select(
            ProfileId::try_new(FLAT_PROFILE).expect("static profile ID is valid"),
            ProfileName::try_new("Flat").expect("static profile name is valid"),
            ProfileSource::Flat,
        );
    }

    fn select_preset(&mut self, index: usize) {
        let Some(preset) = self.presets.get(index).cloned() else {
            self.action_error = Some("preset is no longer available".into());
            return;
        };
        let id = ProfileId::try_new(format!("preset-{index}"))
            .expect("generated profile ID is not empty");
        let name = ProfileName::try_new(format!("{} {}", preset.brand, preset.model))
            .expect("catalog brand and model produce a non-empty name");
        self.select(id, name, ProfileSource::Preset(preset.id));
    }

    fn select(&mut self, id: ProfileId, name: ProfileName, source: ProfileSource) {
        let result = if self.backend.state().profile(&id).is_none() {
            self.backend.execute(Command::CreateProfile {
                id: id.clone(),
                name,
                source,
            })
        } else {
            Ok(self.backend.state().clone())
        }
        .and_then(|_| {
            self.backend.execute(Command::SelectProfile {
                device: self.device.clone(),
                profile: id,
            })
        });
        self.action_error = result
            .err()
            .map(|error| format!("{error:?}"))
            .or_else(|| self.publish_selected_chain());
    }

    fn clear_selection(&mut self) {
        self.action_error = self
            .backend
            .execute(Command::ClearProfile(self.device.clone()))
            .err()
            .map(|error| format!("{error:?}"))
            .or_else(|| self.publish_chain(Chain::default()));
    }

    fn active_chain(&self) -> Chain {
        self.backend
            .state()
            .selected_profile(&self.device)
            .map_or_else(Chain::default, |profile| profile.chain.clone())
    }

    fn publish_selected_chain(&self) -> Option<String> {
        self.publish_chain(self.active_chain())
    }

    fn publish_chain(&self, chain: Chain) -> Option<String> {
        self.controller
            .as_ref()
            .and_then(|controller| controller.set_chain(chain).err())
            .map(|error| format!("publish chain: {error:?}"))
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::{
        Model, animate_meter, animate_spectrum_point, highest_peak, meter_fraction,
        spectrum_fraction,
    };
    use tunic_core::{AudioFormat, Chain, Processor, SampleRateHz};

    #[test]
    fn selecting_profiles_updates_the_authoritative_backend_state() {
        let mut model = Model::new("System Output".into(), None, None);
        assert_eq!(model.selected_profile_name(), "None");

        model.select_preset(1);
        assert_eq!(model.selected_profile_name(), "Sony MDR-7506");

        model.select_flat();
        assert_eq!(model.selected_profile_name(), "Flat");
        assert_eq!(model.backend.state().profiles.len(), 2);

        model.clear_selection();
        assert_eq!(model.selected_profile_name(), "None");
        assert!(model.action_error.is_none());
    }

    #[test]
    fn selecting_a_preset_publishes_its_chain_to_the_processor() {
        let (mut processor, controller) = Processor::new(
            AudioFormat {
                sample_rate: SampleRateHz::try_new(48_000.0).unwrap(),
                maximum_frame_count: NonZeroUsize::new(512).unwrap(),
            },
            Chain::default(),
            false,
        )
        .unwrap();
        let mut model = Model::new("System Output".into(), Some(controller), None);
        let mut samples = vec![0.25_f32; 512 * 2];

        model.select_preset(1);
        processor.process(&mut samples);

        assert!(samples.iter().any(|sample| *sample != 0.25));
        assert!(model.action_error.is_none());
    }

    #[test]
    fn meter_maps_decibels_to_a_clamped_fraction() {
        assert_eq!(meter_fraction(0.0), 0.0);
        assert_eq!(meter_fraction(0.001), 0.0);
        assert!((meter_fraction(0.1) - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(meter_fraction(1.0), 1.0);
        assert_eq!(meter_fraction(2.0), 1.0);
        assert_eq!(meter_fraction(f32::NAN), 0.0);
    }

    #[test]
    fn meter_attacks_immediately_and_decays_at_twenty_four_decibels_per_second() {
        assert_eq!(animate_meter(0.25, Some(1.0), 0.1), 1.0);
        assert!((animate_meter(1.0, None, 0.1) - 0.758_577_6).abs() < 1e-6);
        assert_eq!(animate_meter(1.0, Some(0.9), 0.1), 0.9);
    }

    #[test]
    fn meter_keeps_the_highest_peak_from_unseen_telemetry() {
        let peak = [0.2, 0.9, 0.1]
            .into_iter()
            .fold(None, highest_peak)
            .unwrap();

        assert_eq!(peak, 0.9);
    }

    #[test]
    fn spectrum_maps_decibels_to_a_clamped_fraction() {
        assert_eq!(spectrum_fraction(0.0), 0.0);
        assert_eq!(spectrum_fraction(0.000_01), 0.0);
        assert!((spectrum_fraction(0.001) - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(spectrum_fraction(1.0), 1.0);
        assert_eq!(spectrum_fraction(f32::NAN), 0.0);
    }

    #[test]
    fn spectrum_attacks_immediately_and_decays_for_each_telemetry_frame() {
        let frame_seconds = 1.0 / 60.0;

        assert_eq!(animate_spectrum_point(0.0, 0.5, frame_seconds), 0.5);
        assert_eq!(animate_spectrum_point(0.5, 1.0, frame_seconds), 1.0);
        assert!((animate_spectrum_point(1.0, 0.0, frame_seconds) - 0.926_118_73).abs() < 1e-6);
    }
}
