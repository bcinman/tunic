//! Shared GPUI presentation for Tunic desktop applications.

mod components;

use std::{cell::Cell, rc::Rc};

use gpui::{
    Bounds, Context, Div, Entity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels,
    Point, Render, Stateful, Window, canvas, div, fill, oklcha, point, prelude::*, px, rgb, size,
};
use tunic_core::{
    Backend, Chain, Command, Controller, DeviceId, FilterId, FrequencyHz, GainDb, MemoryStore,
    PresetCatalog, PresetQuery, PresetSummary, Profile, ProfileId, ProfileName, ProfileSource,
    SampleRateHz,
};
use tunic_presets::BundledCatalog;

use components::{
    equalizer::{closest_filter, maximum_frequency_hz, values_at_position},
    shell,
    telemetry::SpectrumView,
};

const SYSTEM_OUTPUT: &str = "system-output";
const FLAT_PROFILE: &str = "flat";
const EQ_FALLBACK_SAMPLE_RATE_HZ: f64 = 48_000.0;
const CONTROL_MIN_GAIN_DB: f64 = -12.0;
const CONTROL_MAX_GAIN_DB: f64 = 12.0;

pub struct TunicView {
    model: Model,
    spectrum: Entity<SpectrumView>,
    dragging_filter: Option<FilterId>,
    dragging_control: Option<FilterId>,
}

impl TunicView {
    pub fn new(
        device_name: String,
        controller: Option<Controller>,
        audio_error: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let spectrum = cx.new(|_| SpectrumView::new(controller.as_ref()));
        Self {
            model: Model::new(device_name, controller, audio_error),
            spectrum,
            dragging_filter: None,
            dragging_control: None,
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
        self.spectrum.update(cx, |view, cx| {
            view.replace_controller(controller.as_ref());
            cx.notify();
        });
        self.model.device_name = device_name;
        self.model.controller = controller;
        self.model.audio_error = audio_error;
    }
}

impl Render for TunicView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.model.selected_profile_name();
        let presets = self.model.presets.clone();
        let chain = self.model.base_chain();
        let controls = self.model.controls();
        let exposed = controls.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
        let sample_rate = self.model.sample_rate();
        let graph_bounds = Rc::new(Cell::new(None));
        let mouse_down_bounds = Rc::clone(&graph_bounds);
        let mouse_move_bounds = Rc::clone(&graph_bounds);

        div()
            .id("tunic-root")
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .pb_5()
            .overflow_scroll()
            .bg(oklcha(0.21, 0.0, 0.0, 0.58))
            .text_color(rgb(0xf2f2f2))
            .border_b_1()
            .border_color(oklcha(1.0, 0.0, 0.0, 0.1))
            .child(
                div()
                    .flex_none()
                    .overflow_hidden()
                    .child(shell::header(self.model.device_name.clone(), selected))
                    .child(components::equalizer::graph(
                        &chain,
                        &exposed,
                        sample_rate,
                        self.spectrum.clone(),
                        Rc::clone(&graph_bounds),
                    )),
            )
            .when(!controls.is_empty(), |view| {
                view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child("Adjustments")
                        .children(controls.into_iter().enumerate().map(
                            |(index, (target, name, gain))| {
                                gain_control(index, target, name, gain, cx)
                            },
                        )),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button("save", "Save").on_click(cx.listener(|this, _, _, cx| {
                            this.model.save_draft();
                            cx.notify();
                        })),
                    )
                    .child(
                        button("reset", "Reset").on_click(cx.listener(|this, _, _, cx| {
                            this.dragging_filter = None;
                            this.model.reset_draft();
                            cx.notify();
                        })),
                    ),
            )
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, _| {
                    let Some(bounds) = mouse_down_bounds.get() else {
                        return;
                    };
                    this.dragging_filter = closest_filter(
                        &this.model.base_chain(),
                        this.model.sample_rate(),
                        bounds,
                        event.position,
                    );
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                let (Some(filter), Some(bounds)) = (this.dragging_filter, mouse_move_bounds.get())
                else {
                    return;
                };
                if !event.dragging() {
                    this.dragging_filter = None;
                    return;
                }
                this.model.drag_filter(filter, bounds, event.position);
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging_filter = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging_filter = None),
            )
    }
}

fn gain_control(
    index: usize,
    target: FilterId,
    name: String,
    gain: f64,
    cx: &mut Context<TunicView>,
) -> Div {
    let fraction = ((gain - CONTROL_MIN_GAIN_DB) / (CONTROL_MAX_GAIN_DB - CONTROL_MIN_GAIN_DB))
        .clamp(0.0, 1.0) as f32;
    let bounds = Rc::new(Cell::new(None));
    let prepaint_bounds = Rc::clone(&bounds);
    let mouse_down_bounds = Rc::clone(&bounds);
    let mouse_move_bounds = Rc::clone(&bounds);

    div()
        .flex()
        .items_center()
        .gap_3()
        .child(div().w(px(205.0)).text_sm().child(name))
        .child(
            div()
                .id(("gain-control", index))
                .relative()
                .h_5()
                .flex_1()
                .cursor_pointer()
                .child(
                    canvas(
                        move |bounds, _, _| prepaint_bounds.set(Some(bounds)),
                        move |bounds, _, window, _| {
                            let track_y = bounds.origin.y + bounds.size.height / 2.0;
                            let center_x = bounds.origin.x + bounds.size.width / 2.0;
                            let thumb_x = bounds.origin.x + bounds.size.width * fraction;
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(bounds.origin.x, track_y - px(2.0)),
                                    size: size(bounds.size.width, px(4.0)),
                                },
                                rgb(0x484b52),
                            ));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(center_x.min(thumb_x), track_y - px(2.0)),
                                    size: size((thumb_x - center_x).abs(), px(4.0)),
                                },
                                rgb(0x69b578),
                            ));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(thumb_x - px(4.0), track_y - px(8.0)),
                                    size: size(px(8.0), px(16.0)),
                                },
                                rgb(0xf2f2f2),
                            ));
                        },
                    )
                    .size_full(),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        let Some(bounds) = mouse_down_bounds.get() else {
                            return;
                        };
                        this.dragging_control = Some(target);
                        this.model.set_control_gain(
                            target,
                            gain_at_slider_position(bounds, event.position),
                        );
                        cx.notify();
                    }),
                )
                .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if this.dragging_control != Some(target) || !event.dragging() {
                        return;
                    }
                    let Some(bounds) = mouse_move_bounds.get() else {
                        return;
                    };
                    this.model
                        .set_control_gain(target, gain_at_slider_position(bounds, event.position));
                    cx.notify();
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.dragging_control = None),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.dragging_control = None),
                ),
        )
        .child(div().w(px(64.0)).text_sm().child(format!("{gain:+.1} dB")))
}

fn gain_at_slider_position(bounds: Bounds<Pixels>, position: Point<Pixels>) -> f64 {
    let fraction = ((position.x - bounds.origin.x) / bounds.size.width).clamp(0.0, 1.0);
    CONTROL_MIN_GAIN_DB + f64::from(fraction) * (CONTROL_MAX_GAIN_DB - CONTROL_MIN_GAIN_DB)
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
    draft: Option<Profile>,
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
            draft: None,
            presets,
            audio_error,
            action_error: None,
        }
    }

    fn selected_profile_name(&self) -> String {
        self.backend
            .state()
            .selected_profile(&self.device)
            .map_or_else(|| "None".into(), |profile| profile.name().to_string())
    }

    fn sample_rate(&self) -> SampleRateHz {
        self.controller.as_ref().map_or_else(
            || {
                SampleRateHz::try_new(EQ_FALLBACK_SAMPLE_RATE_HZ)
                    .expect("fallback sample rate is valid")
            },
            Controller::sample_rate,
        )
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
        self.draft = None;
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
        self.draft = None;
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
            .map_or_else(Chain::default, |profile| {
                self.draft.as_ref().unwrap_or(profile).effective_chain()
            })
    }

    fn base_chain(&self) -> Chain {
        let Some(profile) = self
            .draft
            .as_ref()
            .or_else(|| self.backend.state().selected_profile(&self.device))
        else {
            return Chain::default();
        };
        profile.base().clone()
    }

    fn controls(&self) -> Vec<(FilterId, String, f64)> {
        self.draft
            .as_ref()
            .or_else(|| self.backend.state().selected_profile(&self.device))
            .map_or_else(Vec::new, |profile| {
                profile
                    .controls()
                    .iter()
                    .map(|control| {
                        (
                            control.target(),
                            control.name().to_string(),
                            control.gain_adjustment().into_inner(),
                        )
                    })
                    .collect()
            })
    }

    fn drag_filter(
        &mut self,
        filter_id: FilterId,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
    ) {
        let Some(profile) = self.backend.state().selected_profile(&self.device).cloned() else {
            return;
        };
        let max_frequency = maximum_frequency_hz().min(self.sample_rate().into_inner() * 0.499);
        let (frequency, gain) = values_at_position(bounds, position, max_frequency);
        let mut draft = self.draft.clone().unwrap_or(profile);
        let frequency = FrequencyHz::try_new(frequency).expect("dragged frequency stays positive");
        let gain = GainDb::try_new(gain).expect("dragged gain stays finite");
        let mut base = draft.base().clone();
        let Some(filter) = base
            .equalizer
            .filters
            .iter_mut()
            .find(|filter| filter.id == filter_id)
        else {
            return;
        };
        filter.parameters.frequency = frequency;
        filter.parameters.gain = gain;
        if let Err(error) = draft.replace_base(base) {
            self.action_error = Some(format!("{error:?}"));
            return;
        }

        self.action_error = self.publish_chain(draft.effective_chain());
        if self.action_error.is_none() {
            self.draft = Some(draft);
        }
    }

    fn set_control_gain(&mut self, target: FilterId, gain: f64) {
        let Some(profile) = self.backend.state().selected_profile(&self.device).cloned() else {
            return;
        };
        let mut draft = self.draft.clone().unwrap_or(profile);
        let gain = GainDb::try_new(gain).expect("slider gain is finite");
        if let Err(error) = draft.adjust_filter_gain(target, gain) {
            self.action_error = Some(format!("{error:?}"));
            return;
        }
        self.action_error = self.publish_chain(draft.effective_chain());
        if self.action_error.is_none() {
            self.draft = Some(draft);
        }
    }

    fn save_draft(&mut self) {
        let Some(draft) = self.draft.clone() else {
            return;
        };
        match self.backend.execute(Command::SaveProfile(draft)) {
            Ok(_) => {
                self.draft = None;
                self.action_error = None;
            }
            Err(error) => self.action_error = Some(format!("{error:?}")),
        }
    }

    fn reset_draft(&mut self) {
        if self.draft.is_none() {
            return;
        }
        let saved = self
            .backend
            .state()
            .selected_profile(&self.device)
            .map_or_else(Chain::default, |profile| profile.effective_chain());
        self.action_error = self.publish_chain(saved);
        if self.action_error.is_none() {
            self.draft = None;
        }
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

    use super::{Model, gain_at_slider_position};
    use crate::components::{
        equalizer::{
            frequency_at_fraction, frequency_fraction, gain_at_fraction, gain_fraction,
            graph_point, individual_responses as individual_equalizer_responses,
            response as equalizer_response,
        },
        telemetry::{animate_spectrum_point, spectrum_fraction},
    };
    use gpui::{Bounds, point, px, size};
    use tunic_core::{
        AudioFormat, Chain, FrequencyHz, GainDb, Processor, ProcessorError, SampleRateHz,
    };

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

    #[test]
    fn equalizer_coordinates_are_logarithmic_and_centered_on_zero_db() {
        assert_eq!(frequency_fraction(20.0, 20_000.0), 0.0);
        assert!(
            (frequency_fraction(20_000.0_f64.sqrt() * 20.0_f64.sqrt(), 20_000.0) - 0.5).abs()
                < 1e-6
        );
        assert_eq!(frequency_fraction(20_000.0, 20_000.0), 1.0);
        assert_eq!(gain_fraction(20.0), 0.0);
        assert_eq!(gain_fraction(0.0), 0.5);
        assert_eq!(gain_fraction(-20.0), 1.0);
    }

    #[test]
    fn equalizer_drag_coordinates_invert_the_display_mapping() {
        let middle_frequency = 20_000.0_f64.sqrt() * 20.0_f64.sqrt();
        assert!((frequency_at_fraction(0.5, 20_000.0) - middle_frequency).abs() < 1e-6);
        assert_eq!(gain_at_fraction(0.25), 10.0);
        assert_eq!(gain_at_fraction(0.75), -10.0);
    }

    #[test]
    fn gain_slider_is_centered_on_zero_and_clamped_to_twelve_decibels() {
        let bounds = Bounds {
            origin: point(px(10.0), px(20.0)),
            size: size(px(200.0), px(20.0)),
        };
        assert_eq!(
            gain_at_slider_position(bounds, point(px(10.0), px(20.0))),
            -12.0
        );
        assert_eq!(
            gain_at_slider_position(bounds, point(px(110.0), px(20.0))),
            0.0
        );
        assert_eq!(
            gain_at_slider_position(bounds, point(px(210.0), px(20.0))),
            12.0
        );
        assert_eq!(
            gain_at_slider_position(bounds, point(px(500.0), px(20.0))),
            12.0
        );
    }

    #[test]
    fn incompatible_equalizer_response_is_reported_instead_of_panicking() {
        let mut model = Model::new("System Output".into(), None, None);
        model.select_preset(1);
        let sample_rate = SampleRateHz::try_new(16_000.0).unwrap();

        assert!(matches!(
            equalizer_response(&model.active_chain(), sample_rate, 7_984.0),
            Err(ProcessorError::FilterAtOrAboveNyquist { .. })
        ));
    }

    #[test]
    fn individual_filter_responses_sum_to_the_combined_response() {
        let mut model = Model::new("System Output".into(), None, None);
        model.select_preset(0);
        let chain = model.active_chain();
        let sample_rate = SampleRateHz::try_new(48_000.0).unwrap();
        let combined = equalizer_response(&chain, sample_rate, 20_000.0).unwrap();
        let individual = individual_equalizer_responses(&chain, sample_rate, 20_000.0);

        assert_eq!(individual.len(), chain.equalizer.filters.len());
        for (index, combined) in combined.into_iter().enumerate() {
            let sum = individual
                .iter()
                .map(|response| response[index])
                .sum::<f32>();
            assert!((combined - sum).abs() < 1e-4);
        }
    }

    #[test]
    fn gain_controls_are_live_drafts_until_saved_or_reset() {
        let mut model = Model::new("System Output".into(), None, None);
        model.select_preset(0);
        let saved = model
            .backend
            .state()
            .selected_profile(&model.device)
            .unwrap()
            .clone();
        let filter_id = saved.controls()[0].target();
        model.set_control_gain(filter_id, 4.0);
        let draft = model.active_chain();
        assert_eq!(draft.equalizer.filters[2].parameters.gain.into_inner(), 9.5);
        assert_ne!(draft, saved.effective_chain());
        assert_eq!(
            model
                .backend
                .state()
                .selected_profile(&model.device)
                .unwrap(),
            &saved
        );

        model.reset_draft();
        assert_eq!(model.active_chain(), saved.effective_chain());

        model.set_control_gain(filter_id, 4.0);
        let draft = model.active_chain();
        model.save_draft();
        let persisted = model
            .backend
            .state()
            .selected_profile(&model.device)
            .unwrap();
        assert_eq!(persisted.effective_chain(), draft);
        assert_eq!(persisted.base(), saved.base());
        assert_eq!(
            persisted
                .control(filter_id)
                .unwrap()
                .gain_adjustment()
                .into_inner(),
            4.0
        );
        assert_eq!(persisted.revision().0, saved.revision().0 + 1);
        assert!(model.draft.is_none());
    }

    #[test]
    fn graph_edits_an_exposed_filter_base_without_changing_its_adjustment() {
        let mut model = Model::new("System Output".into(), None, None);
        model.select_preset(0);
        let saved = model
            .backend
            .state()
            .selected_profile(&model.device)
            .unwrap()
            .clone();
        let filter_id = saved.controls()[0].target();
        let bounds = Bounds {
            origin: point(px(10.0), px(20.0)),
            size: size(px(1_000.0), px(200.0)),
        };

        model.drag_filter(filter_id, bounds, graph_point(bounds, 0.75, 0.25));
        model.save_draft();

        let persisted = model
            .backend
            .state()
            .selected_profile(&model.device)
            .unwrap();
        assert_eq!(
            persisted.base().equalizer.filters[2]
                .parameters
                .gain
                .into_inner(),
            10.0
        );
        assert_ne!(persisted.base(), saved.base());
        assert_eq!(
            persisted.control(filter_id).unwrap().gain_adjustment(),
            GainDb::default()
        );
    }

    #[test]
    fn failed_reset_keeps_the_compatible_live_draft() {
        let mut model = Model::new("System Output".into(), None, None);
        model.select_preset(1);
        let mut draft = model.active_chain();
        for filter in &mut draft.equalizer.filters {
            if filter.parameters.frequency.into_inner() >= 8_000.0 {
                filter.parameters.frequency = FrequencyHz::try_new(4_000.0).unwrap();
            }
        }
        let (_, controller) = Processor::new(
            AudioFormat {
                sample_rate: SampleRateHz::try_new(16_000.0).unwrap(),
                maximum_frame_count: NonZeroUsize::new(512).unwrap(),
            },
            draft.clone(),
            false,
        )
        .unwrap();
        model.controller = Some(controller);
        let mut profile = model
            .backend
            .state()
            .selected_profile(&model.device)
            .unwrap()
            .clone();
        profile.replace_base(draft.clone()).unwrap();
        model.draft = Some(profile);

        model.reset_draft();

        assert_eq!(model.active_chain(), draft);
        assert!(model.action_error.is_some());
    }

    #[test]
    fn saving_does_not_republish_and_reset_filter_history() {
        let mut model = Model::new("System Output".into(), None, None);
        model.select_preset(0);
        let mut draft = model.active_chain();
        draft.equalizer.filters[0].parameters.gain = GainDb::try_new(8.0).unwrap();
        let format = || AudioFormat {
            sample_rate: SampleRateHz::try_new(48_000.0).unwrap(),
            maximum_frame_count: NonZeroUsize::new(512).unwrap(),
        };
        let (mut processor, controller) = Processor::new(format(), draft.clone(), false).unwrap();
        let (mut reference, _) = Processor::new(format(), draft.clone(), false).unwrap();
        model.controller = Some(controller);
        let mut profile = model
            .backend
            .state()
            .selected_profile(&model.device)
            .unwrap()
            .clone();
        profile.replace_base(draft).unwrap();
        model.draft = Some(profile);

        let mut warmup = vec![0.25_f32; 512 * 2];
        let mut reference_warmup = warmup.clone();
        processor.process(&mut warmup);
        reference.process(&mut reference_warmup);
        model.save_draft();
        let mut actual = vec![0.25_f32; 512 * 2];
        let mut expected = actual.clone();
        processor.process(&mut actual);
        reference.process(&mut expected);

        assert_eq!(actual, expected);
        assert!(model.action_error.is_none());
    }
}
