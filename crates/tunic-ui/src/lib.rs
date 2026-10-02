//! Shared GPUI presentation for Tunic desktop applications.

use std::{cell::Cell, collections::VecDeque, rc::Rc, time::Instant};

use gpui::{
    Bounds, Context, Div, Entity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    PathBuilder, Pixels, Point, Render, Rgba, Stateful, Window, canvas, div, fill, oklcha, point,
    prelude::*, px, relative, rgb, size,
};
use tunic_core::{
    Backend, Chain, Command, Controller, DeviceId, FilterId, FrequencyHz, FrequencyResponse,
    GainDb, MemoryStore, PresetCatalog, PresetQuery, PresetSummary, ProcessorError, Profile,
    ProfileId, ProfileName, ProfileSource, SPECTRUM_POINT_COUNT, SampleRateHz, Spectrum,
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
const EQ_MIN_FREQUENCY_HZ: f64 = 20.0;
const EQ_MAX_FREQUENCY_HZ: f64 = 20_000.0;
const EQ_MIN_GAIN_DB: f64 = -20.0;
const EQ_MAX_GAIN_DB: f64 = 20.0;
const EQ_FALLBACK_SAMPLE_RATE_HZ: f64 = 48_000.0;
const EQ_POINT_RADIUS_PX: f32 = 4.0;
const EQ_POINT_INSET_PX: f32 = EQ_POINT_RADIUS_PX + 1.0;
const EQ_POINT_HIT_RADIUS_PX: f32 = 12.0;
const CONTROL_MIN_GAIN_DB: f64 = -12.0;
const CONTROL_MAX_GAIN_DB: f64 = 12.0;
const FRAME_HISTORY_LENGTH: usize = 90;
const SLOW_FRAME_MILLISECONDS: f32 = 33.0;

pub struct TunicView {
    model: Model,
    telemetry: Entity<TelemetryView>,
    dragging_filter: Option<FilterId>,
    dragging_control: Option<FilterId>,
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
        let chain = self.model.base_chain();
        let controls = self.model.controls();
        let exposed = controls.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
        let sample_rate = self.model.sample_rate();
        let graph_bounds = Rc::new(Cell::new(None));
        let mouse_down_bounds = Rc::clone(&graph_bounds);
        let mouse_move_bounds = Rc::clone(&graph_bounds);

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_6()
            .bg(oklcha(0.21, 0.0, 0.0, 0.7))
            .text_color(rgb(0xf2f2f2))
            .child(div().pl(px(68.0)).text_xl().child("Tunic"))
            .child(format!("Device: {}", self.model.device_name))
            .child(format!("Selected profile: {selected}"))
            .child(div().text_sm().text_color(rgb(0xb8bbc2)).child("Base EQ"))
            .child(equalizer_graph(
                &chain,
                &exposed,
                sample_rate,
                Rc::clone(&graph_bounds),
            ))
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

fn equalizer_graph(
    chain: &Chain,
    exposed: &[FilterId],
    sample_rate: SampleRateHz,
    graph_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
) -> Div {
    let max_frequency = EQ_MAX_FREQUENCY_HZ.min(sample_rate.into_inner() * 0.499);
    let (response, response_error) = match equalizer_response(chain, sample_rate, max_frequency) {
        Ok(response) => (response, None),
        Err(error) => (Vec::new(), Some(format!("Response unavailable: {error:?}"))),
    };
    let filter_responses = individual_equalizer_responses(chain, sample_rate, max_frequency);
    let points = chain
        .equalizer
        .filters
        .iter()
        .map(|filter| {
            (
                frequency_fraction(filter.parameters.frequency.into_inner(), max_frequency),
                gain_fraction(filter.parameters.gain.into_inner()),
                exposed.contains(&filter.id),
            )
        })
        .collect::<Vec<_>>();
    let max_frequency_label = if max_frequency >= 1_000.0 {
        format!("{:.0} kHz", max_frequency / 1_000.0)
    } else {
        format!("{max_frequency:.0} Hz")
    };

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child("Equalizer")
        .child(
            div()
                .relative()
                .h_40()
                .bg(rgb(0x35373c))
                .child(
                    canvas(
                        move |bounds, _, _| graph_bounds.set(Some(bounds)),
                        move |bounds, _, window, _| {
                            let zero_y = bounds.origin.y + bounds.size.height * gain_fraction(0.0);
                            window.paint_quad(fill(
                                gpui::Bounds {
                                    origin: point(bounds.origin.x, zero_y),
                                    size: size(bounds.size.width, px(1.0)),
                                },
                                rgb(0x5a5d64),
                            ));

                            for filter_response in &filter_responses {
                                paint_equalizer_response(
                                    bounds,
                                    filter_response,
                                    px(1.0),
                                    rgb(0x7b7e85),
                                    window,
                                );
                            }
                            paint_equalizer_response(
                                bounds,
                                &response,
                                px(2.0),
                                rgb(0x69b578),
                                window,
                            );

                            for (x, y, tweakable) in &points {
                                let point_radius = px(EQ_POINT_RADIUS_PX);
                                let center = graph_point(bounds, *x, *y);
                                window.paint_quad(fill(
                                    gpui::Bounds {
                                        origin: point(
                                            center.x - point_radius,
                                            center.y - point_radius,
                                        ),
                                        size: size(point_radius * 2.0, point_radius * 2.0),
                                    },
                                    if *tweakable {
                                        rgb(0xf2f2f2)
                                    } else {
                                        rgb(0x8b8e95)
                                    },
                                ));
                            }
                        },
                    )
                    .size_full(),
                )
                .child(div().absolute().top_1().left_1().text_sm().child("+20 dB"))
                .child(
                    div()
                        .absolute()
                        .bottom_1()
                        .left_1()
                        .text_sm()
                        .child("−20 dB"),
                ),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .text_sm()
                .child("20 Hz")
                .child(max_frequency_label),
        )
        .when_some(response_error, |graph, error| {
            graph.child(div().text_color(rgb(0xff8a8a)).child(error))
        })
}

fn equalizer_response(
    chain: &Chain,
    sample_rate: SampleRateHz,
    max_frequency: f64,
) -> Result<Vec<f32>, ProcessorError> {
    let prepared = FrequencyResponse::new(chain, sample_rate)?;
    (0..SPECTRUM_POINT_COUNT)
        .map(|index| {
            let fraction = index as f64 / (SPECTRUM_POINT_COUNT - 1) as f64;
            let frequency =
                EQ_MIN_FREQUENCY_HZ * (max_frequency / EQ_MIN_FREQUENCY_HZ).powf(fraction);
            let frequency = FrequencyHz::try_new(frequency).expect("graph frequency is positive");
            prepared.db_at(frequency).map(|response| response as f32)
        })
        .collect()
}

fn individual_equalizer_responses(
    chain: &Chain,
    sample_rate: SampleRateHz,
    max_frequency: f64,
) -> Vec<Vec<f32>> {
    chain
        .equalizer
        .filters
        .iter()
        .filter_map(|filter| {
            let mut filter_chain = Chain::default();
            filter_chain.equalizer.filters.push(*filter);
            equalizer_response(&filter_chain, sample_rate, max_frequency).ok()
        })
        .collect()
}

fn paint_equalizer_response(
    bounds: Bounds<Pixels>,
    response: &[f32],
    stroke_width: Pixels,
    color: Rgba,
    window: &mut Window,
) {
    if response.is_empty() {
        return;
    }
    let mut path = PathBuilder::stroke(stroke_width);
    for (index, response) in response.iter().copied().enumerate() {
        let x = bounds.origin.x
            + bounds.size.width * (index as f32 / (SPECTRUM_POINT_COUNT - 1) as f32);
        let y = bounds.origin.y + bounds.size.height * gain_fraction(f64::from(response));
        if index == 0 {
            path.move_to(point(x, y));
        } else {
            path.line_to(point(x, y));
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn frequency_fraction(frequency: f64, max_frequency: f64) -> f32 {
    ((frequency.clamp(EQ_MIN_FREQUENCY_HZ, max_frequency) / EQ_MIN_FREQUENCY_HZ).ln()
        / (max_frequency / EQ_MIN_FREQUENCY_HZ).ln()) as f32
}

fn gain_fraction(gain_db: f64) -> f32 {
    ((EQ_MAX_GAIN_DB - gain_db.clamp(EQ_MIN_GAIN_DB, EQ_MAX_GAIN_DB))
        / (EQ_MAX_GAIN_DB - EQ_MIN_GAIN_DB)) as f32
}

fn frequency_at_fraction(fraction: f32, max_frequency: f64) -> f64 {
    EQ_MIN_FREQUENCY_HZ
        * (max_frequency / EQ_MIN_FREQUENCY_HZ).powf(f64::from(fraction.clamp(0.0, 1.0)))
}

fn gain_at_fraction(fraction: f32) -> f64 {
    EQ_MAX_GAIN_DB - f64::from(fraction.clamp(0.0, 1.0)) * (EQ_MAX_GAIN_DB - EQ_MIN_GAIN_DB)
}

fn graph_point(bounds: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    let inset = px(EQ_POINT_INSET_PX);
    point(
        bounds.origin.x + inset + (bounds.size.width - inset * 2.0) * x,
        bounds.origin.y + inset + (bounds.size.height - inset * 2.0) * y,
    )
}

fn graph_fractions(bounds: Bounds<Pixels>, position: Point<Pixels>) -> (f32, f32) {
    let inset = px(EQ_POINT_INSET_PX);
    let width = bounds.size.width - inset * 2.0;
    let height = bounds.size.height - inset * 2.0;
    (
        ((position.x - bounds.origin.x - inset) / width).clamp(0.0, 1.0),
        ((position.y - bounds.origin.y - inset) / height).clamp(0.0, 1.0),
    )
}

fn closest_filter(
    chain: &Chain,
    sample_rate: SampleRateHz,
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
) -> Option<FilterId> {
    let max_frequency = EQ_MAX_FREQUENCY_HZ.min(sample_rate.into_inner() * 0.499);
    chain
        .equalizer
        .filters
        .iter()
        .filter_map(|filter| {
            let center = graph_point(
                bounds,
                frequency_fraction(filter.parameters.frequency.into_inner(), max_frequency),
                gain_fraction(filter.parameters.gain.into_inner()),
            );
            let x = (position.x - center.x).as_f32();
            let y = (position.y - center.y).as_f32();
            let distance_squared = x.mul_add(x, y * y);
            (distance_squared <= EQ_POINT_HIT_RADIUS_PX.powi(2))
                .then_some((filter.id, distance_squared))
        })
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(id, _)| id)
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
        let (x, y) = graph_fractions(bounds, position);
        let max_frequency = EQ_MAX_FREQUENCY_HZ.min(self.sample_rate().into_inner() * 0.499);
        let mut draft = self.draft.clone().unwrap_or(profile);
        let frequency = FrequencyHz::try_new(frequency_at_fraction(x, max_frequency))
            .expect("dragged frequency stays positive");
        let gain = GainDb::try_new(gain_at_fraction(y)).expect("dragged gain stays finite");
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

    use super::{
        Model, animate_meter, animate_spectrum_point, equalizer_response, frequency_at_fraction,
        frequency_fraction, gain_at_fraction, gain_at_slider_position, gain_fraction, graph_point,
        highest_peak, individual_equalizer_responses, meter_fraction, spectrum_fraction,
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
