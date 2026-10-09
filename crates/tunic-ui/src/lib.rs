//! Shared GPUI presentation for Tunic desktop applications.

mod components;

use std::{cell::Cell, rc::Rc};

use gpui::{
    Bounds, Context, Div, Entity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels,
    Point, Render, Stateful, Window, canvas, div, fill, oklcha, point, prelude::*, px, rgb, size,
};
use tunic_core::{Command, FilterId, FrequencyHz, GainDb, Session};

use components::{
    equalizer::{closest_filter, maximum_frequency_hz, values_at_position},
    shell,
    telemetry::SpectrumView,
};

const CONTROL_MIN_GAIN_DB: f64 = -12.0;
const CONTROL_MAX_GAIN_DB: f64 = 12.0;

pub struct TunicView {
    session: Session,
    spectrum: Entity<SpectrumView>,
    dragging_filter: Option<FilterId>,
    dragging_control: Option<FilterId>,
}

impl TunicView {
    pub fn new(session: Session, cx: &mut Context<Self>) -> Self {
        let spectrum = cx.new(|_| SpectrumView::new(session.subscribe_telemetry()));
        Self {
            session,
            spectrum,
            dragging_filter: None,
            dragging_control: None,
        }
    }

    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn execute(&mut self, command: Command, cx: &mut Context<Self>) {
        let generation = self.session.audio_generation();
        let _ = self.session.execute(command);
        if generation != self.session.audio_generation() {
            self.spectrum.update(cx, |view, cx| {
                view.replace_telemetry(self.session.subscribe_telemetry());
                cx.notify();
            });
        }
        cx.notify();
    }
}

impl Render for TunicView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self
            .session
            .selected_profile()
            .map_or_else(|| "None".into(), |profile| profile.name().to_string());
        let presets = self.session.presets();
        let chain = self.session.base_chain();
        let controls = self
            .session
            .editing_profile()
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
            });
        let exposed = controls.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
        let sample_rate = self.session.sample_rate();
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
                    .child(shell::header(
                        self.session.device_name().to_owned(),
                        selected,
                    ))
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
                            this.execute(Command::SaveDraft, cx);
                        })),
                    )
                    .child(
                        button("reset", "Reset").on_click(cx.listener(|this, _, _, cx| {
                            this.dragging_filter = None;
                            this.execute(Command::ResetDraft, cx);
                        })),
                    ),
            )
            .child(
                button("flat", "Use Flat").on_click(cx.listener(|this, _, _, cx| {
                    this.execute(Command::UseFlat, cx);
                })),
            )
            .children(presets.into_iter().enumerate().map(|(index, preset)| {
                let label = format!("Use {} {} — {}", preset.brand, preset.model, preset.target);
                button(("preset", index), label).on_click(cx.listener(move |this, _, _, cx| {
                    this.execute(Command::UsePreset(preset.id.clone()), cx);
                }))
            }))
            .child(
                button("clear", "Clear selection").on_click(cx.listener(|this, _, _, cx| {
                    this.execute(Command::ClearSelection, cx);
                })),
            )
            .when_some(self.session.audio_error(), |view, error| {
                view.child(
                    div()
                        .text_color(rgb(0xff8a8a))
                        .child(format!("Audio unavailable: {error}")),
                )
            })
            .when_some(self.session.action_error(), |view, error| {
                view.child(
                    div()
                        .text_color(rgb(0xff8a8a))
                        .child(format!("Error: {error:?}")),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, _| {
                    let Some(bounds) = mouse_down_bounds.get() else {
                        return;
                    };
                    this.dragging_filter = closest_filter(
                        &this.session.base_chain(),
                        this.session.sample_rate(),
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
                let max_frequency =
                    maximum_frequency_hz().min(this.session.sample_rate().into_inner() * 0.499);
                let (frequency, gain) = values_at_position(bounds, event.position, max_frequency);
                this.execute(
                    Command::EditFilter {
                        filter,
                        frequency: FrequencyHz::try_new(frequency)
                            .expect("drag frequency is positive"),
                        gain: GainDb::try_new(gain).expect("drag gain is finite"),
                    },
                    cx,
                );
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
                        this.execute(
                            Command::PreviewControlGain {
                                filter: target,
                                gain: GainDb::try_new(gain_at_slider_position(
                                    bounds,
                                    event.position,
                                ))
                                .expect("slider gain is finite"),
                            },
                            cx,
                        );
                    }),
                )
                .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if this.dragging_control != Some(target) || !event.dragging() {
                        return;
                    }
                    let Some(bounds) = mouse_move_bounds.get() else {
                        return;
                    };
                    this.execute(
                        Command::PreviewControlGain {
                            filter: target,
                            gain: GainDb::try_new(gain_at_slider_position(bounds, event.position))
                                .expect("slider gain is finite"),
                        },
                        cx,
                    );
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        if this.dragging_control.take().is_some() {
                            this.execute(Command::FinishControlGain, cx);
                        }
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        if this.dragging_control.take().is_some() {
                            this.execute(Command::FinishControlGain, cx);
                        }
                    }),
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

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::sync::Arc;

    use super::gain_at_slider_position;
    use crate::components::{
        equalizer::{
            frequency_at_fraction, frequency_fraction, gain_at_fraction, gain_fraction,
            graph_point, individual_responses as individual_equalizer_responses,
            response as equalizer_response, values_at_position,
        },
        telemetry::{animate_spectrum_point, spectrum_fraction},
    };
    use gpui::{Bounds, point, px, size};
    use tunic_core::{
        AudioFormat, Chain, ChangeHandler, Command, Connection, FrequencyHz, GainDb,
        MemoryPersistence, Platform, Processor, ProcessorError, SampleRateHz, Session,
    };
    use tunic_presets::BundledCatalog;

    struct TestPlatform(Option<Connection>);

    impl Platform for TestPlatform {
        fn watch_default_output(&mut self, _: ChangeHandler) -> Result<(), String> {
            Ok(())
        }

        fn refresh_default_output(&mut self, _: &Chain) -> Result<Option<Connection>, String> {
            Ok(self.0.take())
        }
    }

    fn session_with_preset(index: usize) -> Session {
        let mut session = Session::new(MemoryPersistence::default(), BundledCatalog).unwrap();
        session
            .execute(Command::UsePreset(session.presets()[index].id.clone()))
            .unwrap();
        session
    }

    #[test]
    fn selecting_profiles_updates_the_authoritative_session_state() {
        let mut session = session_with_preset(1);
        assert_eq!(
            session.selected_profile().unwrap().name().to_string(),
            "Sony MDR-7506"
        );
        session.execute(Command::UseFlat).unwrap();
        assert_eq!(
            session.selected_profile().unwrap().name().to_string(),
            "Flat"
        );
        assert_eq!(session.state().profiles.len(), 2);
        session.execute(Command::ClearSelection).unwrap();
        assert!(session.selected_profile().is_none());
        assert!(session.action_error().is_none());
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
        let mut session = Session::new(MemoryPersistence::default(), BundledCatalog)
            .unwrap()
            .with_audio(
                TestPlatform(Some(Connection {
                    device_name: "Test".into(),
                    controller,
                })),
                Arc::new(|| {}),
            );
        let mut samples = vec![0.25_f32; 512 * 2];

        session
            .execute(Command::UsePreset(session.presets()[1].id.clone()))
            .unwrap();
        processor.process(&mut samples);

        assert!(samples.iter().any(|sample| *sample != 0.25));
        assert!(session.action_error().is_none());
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
        let session = session_with_preset(1);
        let sample_rate = SampleRateHz::try_new(16_000.0).unwrap();

        assert!(matches!(
            equalizer_response(&session.active_chain(), sample_rate, 7_984.0),
            Err(ProcessorError::FilterAtOrAboveNyquist { .. })
        ));
    }

    #[test]
    fn individual_filter_responses_sum_to_the_combined_response() {
        let chain = session_with_preset(0).active_chain();
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
    fn gain_controls_persist_immediately_without_changing_base() {
        let mut session = session_with_preset(0);
        let saved = session.selected_profile().unwrap().clone();
        let filter_id = saved.controls()[0].target();
        let edit = Command::SetControlGain {
            filter: filter_id,
            gain: GainDb::try_new(4.0).unwrap(),
        };
        session.execute(edit).unwrap();
        let active = session.active_chain();
        assert_eq!(
            active.equalizer.filters[2].parameters.gain.into_inner(),
            9.5
        );
        let persisted = session.selected_profile().unwrap();
        assert_eq!(persisted.effective_chain(), active);
        assert_eq!(persisted.base(), saved.base());
        assert_eq!(
            persisted
                .control(filter_id)
                .unwrap()
                .gain_adjustment()
                .into_inner(),
            4.0
        );
        assert!(session.draft().is_none());
    }

    #[test]
    fn graph_edits_an_exposed_filter_base_without_changing_its_adjustment() {
        let mut session = session_with_preset(0);
        let saved = session.selected_profile().unwrap().clone();
        let filter_id = saved.controls()[0].target();
        let bounds = Bounds {
            origin: point(px(10.0), px(20.0)),
            size: size(px(1_000.0), px(200.0)),
        };

        let (frequency, gain) =
            values_at_position(bounds, graph_point(bounds, 0.75, 0.25), 20_000.0);
        session
            .execute(Command::EditFilter {
                filter: filter_id,
                frequency: FrequencyHz::try_new(frequency).unwrap(),
                gain: GainDb::try_new(gain).unwrap(),
            })
            .unwrap();
        session.execute(Command::SaveDraft).unwrap();
        let persisted = session.selected_profile().unwrap();
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
        let mut session = session_with_preset(1);
        for filter in session.active_chain().equalizer.filters {
            if filter.parameters.frequency.into_inner() >= 8_000.0 {
                session
                    .execute(Command::EditFilter {
                        filter: filter.id,
                        frequency: FrequencyHz::try_new(4_000.0).unwrap(),
                        gain: filter.parameters.gain,
                    })
                    .unwrap();
            }
        }
        let draft = session.active_chain();
        let (_, controller) = Processor::new(
            AudioFormat {
                sample_rate: SampleRateHz::try_new(16_000.0).unwrap(),
                maximum_frame_count: NonZeroUsize::new(512).unwrap(),
            },
            draft.clone(),
            false,
        )
        .unwrap();
        let mut session = session.with_audio(
            TestPlatform(Some(Connection {
                device_name: "Test".into(),
                controller,
            })),
            Arc::new(|| {}),
        );
        assert!(session.execute(Command::ResetDraft).is_err());
        assert_eq!(session.active_chain(), draft);
        assert!(session.action_error().is_some());
    }

    #[test]
    fn saving_does_not_republish_and_reset_filter_history() {
        let mut session = session_with_preset(0);
        let filter = session.active_chain().equalizer.filters[0];
        session
            .execute(Command::EditFilter {
                filter: filter.id,
                frequency: filter.parameters.frequency,
                gain: GainDb::try_new(8.0).unwrap(),
            })
            .unwrap();
        let draft = session.active_chain();
        let format = || AudioFormat {
            sample_rate: SampleRateHz::try_new(48_000.0).unwrap(),
            maximum_frame_count: NonZeroUsize::new(512).unwrap(),
        };
        let (mut processor, controller) = Processor::new(format(), draft.clone(), false).unwrap();
        let (mut reference, _) = Processor::new(format(), draft.clone(), false).unwrap();
        let mut session = session.with_audio(
            TestPlatform(Some(Connection {
                device_name: "Test".into(),
                controller,
            })),
            Arc::new(|| {}),
        );

        let mut warmup = vec![0.25_f32; 512 * 2];
        let mut reference_warmup = warmup.clone();
        processor.process(&mut warmup);
        reference.process(&mut reference_warmup);
        session.execute(Command::SaveDraft).unwrap();
        let mut actual = vec![0.25_f32; 512 * 2];
        let mut expected = actual.clone();
        processor.process(&mut actual);
        reference.process(&mut expected);

        assert_eq!(actual, expected);
        assert!(session.action_error().is_none());
    }
}
