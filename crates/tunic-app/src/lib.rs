//! GPUI application lifecycle and presentation, wiring `tunic-engine` to the
//! `tunic-macos` platform implementation.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, Entity, FontWeight, IntoElement, KeyBinding, Menu, MenuItem, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Render, Task, Window,
    WindowBounds, WindowOptions, actions, canvas, div, point, prelude::*, px, relative, rgb, rgba,
    size,
};
use tunic_dsp::{Equalizer, Filter, FilterKind, FrequencyHz, GainDb, PreparedGraph, QualityFactor};
use tunic_engine::{
    EditRevision, Engine, EngineHandle, EngineOptions, EngineSnapshot, EngineStatus,
    SPECTRUM_POINT_COUNT, Spectrum, TelemetryReader,
};
use tunic_macos::CoreAudioPlatform;

actions!(tunic, [CloseWindow, Quit, ShowWindow]);

pub struct AppConfig {
    pub data_directory: PathBuf,
}

#[derive(Debug)]
pub struct AppError(String);

impl AppError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for AppError {}

pub fn default_data_directory() -> Result<PathBuf, AppError> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| AppError::new("HOME is not set; a data directory is required"))?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Tunic"))
}

pub fn run(config: AppConfig) -> Result<(), AppError> {
    let engine = Engine::start(
        EngineOptions {
            database_path: Some(config.data_directory.join("tunic.sqlite3")),
            ..EngineOptions::default()
        },
        CoreAudioPlatform::new,
    )
    .map_err(|error| AppError::new(format!("start engine: {error}")))?;

    let application = gpui_platform::application();
    let app_state = Rc::new(RefCell::new(None));
    let reopen_state = Rc::clone(&app_state);
    application.on_reopen(move |cx| {
        let state = reopen_state.borrow().clone();
        if let Some(state) = state
            && let Err(error) = show_main_window(cx, state)
        {
            eprintln!("error: reopen window: {error}");
        }
    });

    application.run(move |cx: &mut App| {
        let state = cx.new(|cx| TunicView::new(engine, cx));
        *app_state.borrow_mut() = Some(state.clone());

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &CloseWindow, cx| {
            if let Some(window) = cx.windows().into_iter().next() {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        });
        let show_state = state.clone();
        cx.on_action(move |_: &ShowWindow, cx| {
            if let Err(error) = show_main_window(cx, show_state.clone()) {
                eprintln!("error: show window: {error}");
            }
        });
        cx.bind_keys([
            KeyBinding::new("cmd-w", CloseWindow, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.set_menus(vec![
            Menu {
                name: "Tunic".into(),
                items: vec![MenuItem::action("Quit Tunic", Quit)],
                disabled: false,
            },
            Menu {
                name: "Window".into(),
                items: vec![
                    MenuItem::action("Close Window", CloseWindow),
                    MenuItem::action("Show Tunic", ShowWindow),
                ],
                disabled: false,
            },
        ]);

        if let Err(error) = show_main_window(cx, state) {
            eprintln!("error: open window: {error}");
            std::process::exit(1);
        }
        cx.activate(true);
    });
    Ok(())
}

fn show_main_window(cx: &mut App, state: Entity<TunicView>) -> gpui::Result<()> {
    if let Some(window) = cx.windows().into_iter().next() {
        window.update(cx, |_, window, _| window.activate_window())?;
        cx.activate(true);
        return Ok(());
    }

    let bounds = Bounds::centered(None, size(px(980.), px(720.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Tunic".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |_, _| state,
    )?;
    cx.activate(true);
    Ok(())
}

struct TunicView {
    engine: Option<EngineHandle>,
    snapshot: EngineSnapshot,
    spectrum: Entity<SpectrumView>,
    graph_bounds: Rc<Cell<Bounds<Pixels>>>,
    dragging_filter: Option<(usize, EditRevision)>,
    pending_drag_preview: Option<DragPreview>,
    selected_filter: Option<usize>,
    editor_error: Option<String>,
    _refresh_task: Task<()>,
    _drag_preview_task: Task<()>,
}

struct SpectrumView {
    telemetry: TelemetryReader,
    spectrum: Spectrum,
    _refresh_task: Task<()>,
}

impl SpectrumView {
    fn new(telemetry: TelemetryReader, cx: &mut Context<Self>) -> Self {
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

#[derive(Clone, Copy)]
struct DragPreview {
    index: usize,
    filter: Filter,
    revision: EditRevision,
}

impl DragPreview {
    fn apply(self, equalizer: &Equalizer, revision: EditRevision) -> Option<Equalizer> {
        if self.revision != revision {
            return None;
        }
        let mut filters = equalizer.filters().to_vec();
        *filters.get_mut(self.index)? = self.filter;
        Some(Equalizer::with_filters(filters))
    }
}

impl TunicView {
    fn new(engine: EngineHandle, cx: &mut Context<Self>) -> Self {
        let snapshot = engine.snapshot();
        let spectrum = cx.new(|cx| SpectrumView::new(engine.subscribe_telemetry(), cx));
        let refresh_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        let Some(engine) = &this.engine else {
                            return;
                        };
                        let snapshot = engine.snapshot();
                        if snapshot != this.snapshot {
                            if snapshot.edit_revision != this.snapshot.edit_revision {
                                this.cancel_drag();
                            }
                            this.snapshot = snapshot;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let drag_preview_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(32))
                    .await;
                if this
                    .update(cx, |this, cx| this.flush_pending_drag_preview(cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        cx.on_app_quit(|this, _cx| {
            let result = this.engine.take().map(EngineHandle::shutdown);
            async move {
                if let Some(Err(error)) = result {
                    eprintln!("error: shut down engine: {error}");
                }
            }
        })
        .detach();
        Self {
            engine: Some(engine),
            snapshot,
            spectrum,
            graph_bounds: Rc::new(Cell::new(Bounds::default())),
            dragging_filter: None,
            pending_drag_preview: None,
            selected_filter: None,
            editor_error: None,
            _refresh_task: refresh_task,
            _drag_preview_task: drag_preview_task,
        }
    }

    fn toggle_bypass(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = &self.engine else {
            return;
        };
        engine.toggle_bypass();
        self.snapshot = engine.snapshot();
        cx.notify();
    }

    fn preview(&mut self, equalizer: Equalizer, cx: &mut Context<Self>) {
        self.preview_at_revision(equalizer, self.snapshot.edit_revision, cx);
    }

    fn preview_at_revision(
        &mut self,
        equalizer: Equalizer,
        expected_revision: EditRevision,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = &self.engine else {
            return;
        };
        match engine.preview_equalizer(equalizer, expected_revision) {
            Ok(revision) => {
                self.snapshot = engine.snapshot();
                self.editor_error = None;
                if self.snapshot.edit_revision == revision {
                    if let Some((_, drag_revision)) = &mut self.dragging_filter
                        && *drag_revision == expected_revision
                    {
                        *drag_revision = revision;
                    }
                } else {
                    self.cancel_drag();
                }
            }
            Err(error) => {
                self.snapshot = engine.snapshot();
                self.editor_error = Some(error.to_string());
                self.cancel_drag();
            }
        }
        cx.notify();
    }

    fn flush_pending_drag_preview(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.pending_drag_preview.take() else {
            return;
        };
        let Some(equalizer) = preview.apply(&self.snapshot.equalizer, self.snapshot.edit_revision)
        else {
            self.cancel_drag();
            cx.notify();
            return;
        };
        self.preview_at_revision(equalizer, preview.revision, cx);
    }

    fn cancel_drag(&mut self) {
        self.dragging_filter = None;
        self.pending_drag_preview = None;
    }

    fn add_filter(&mut self, cx: &mut Context<Self>) {
        let mut filters = self.snapshot.equalizer.filters().to_vec();
        filters.push(make_filter(FilterKind::Peaking, 1_000.0, 0.0, 1.0));
        self.selected_filter = Some(filters.len() - 1);
        self.preview(Equalizer::with_filters(filters), cx);
    }

    fn replace_filter(&mut self, index: usize, filter: Filter, cx: &mut Context<Self>) {
        let mut filters = self.snapshot.equalizer.filters().to_vec();
        let Some(current) = filters.get_mut(index) else {
            return;
        };
        *current = filter;
        self.preview(Equalizer::with_filters(filters), cx);
    }

    fn remove_filter(&mut self, index: usize, cx: &mut Context<Self>) {
        let mut filters = self.snapshot.equalizer.filters().to_vec();
        if index >= filters.len() {
            return;
        }
        filters.remove(index);
        self.selected_filter = self
            .selected_filter
            .and_then(|selected| selected.checked_sub(usize::from(selected >= index)))
            .filter(|&selected| selected < filters.len());
        self.preview(Equalizer::with_filters(filters), cx);
    }

    fn cycle_filter_kind(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(filter) = self.snapshot.equalizer.filters().get(index).copied() else {
            return;
        };
        let kind = match filter.kind() {
            FilterKind::Peaking => FilterKind::LowShelf,
            FilterKind::LowShelf => FilterKind::HighShelf,
            FilterKind::HighShelf => FilterKind::Peaking,
        };
        self.replace_filter(
            index,
            make_filter(
                kind,
                filter.frequency().get(),
                filter.gain().get(),
                filter.quality_factor().get(),
            ),
            cx,
        );
    }

    fn adjust_filter(
        &mut self,
        index: usize,
        frequency_multiplier: f64,
        gain_delta: f64,
        q_multiplier: f64,
        cx: &mut Context<Self>,
    ) {
        let Some(filter) = self.snapshot.equalizer.filters().get(index).copied() else {
            return;
        };
        self.replace_filter(
            index,
            make_filter(
                filter.kind(),
                (filter.frequency().get() * frequency_multiplier)
                    .clamp(MIN_FREQUENCY_HZ, MAX_FREQUENCY_HZ),
                (filter.gain().get() + gain_delta).clamp(MIN_GAIN_DB, MAX_GAIN_DB),
                (filter.quality_factor().get() * q_multiplier).clamp(MIN_Q, MAX_Q),
            ),
            cx,
        );
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.flush_pending_drag_preview(cx);
        let Some(engine) = &self.engine else {
            return;
        };
        match engine.save_equalizer(self.snapshot.edit_revision) {
            Ok(_) => self.editor_error = None,
            Err(error) => self.editor_error = Some(error.to_string()),
        }
        self.snapshot = engine.snapshot();
        cx.notify();
    }

    fn revert(&mut self, cx: &mut Context<Self>) {
        self.pending_drag_preview = None;
        let Some(engine) = &self.engine else {
            return;
        };
        match engine.discard_preview(self.snapshot.edit_revision) {
            Ok(_) => self.editor_error = None,
            Err(error) => self.editor_error = Some(error.to_string()),
        }
        self.snapshot = engine.snapshot();
        self.dragging_filter = None;
        self.selected_filter = None;
        cx.notify();
    }

    fn begin_graph_drag(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bounds = self.graph_bounds.get();
        let selected = self
            .snapshot
            .equalizer
            .filters()
            .iter()
            .enumerate()
            .filter_map(|(index, filter)| {
                let position = filter_position(bounds, *filter);
                let dx = (position.x - event.position.x).as_f32();
                let dy = (position.y - event.position.y).as_f32();
                let distance_squared = dx * dx + dy * dy;
                (distance_squared <= 18.0 * 18.0).then_some((index, distance_squared))
            })
            .min_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index);
        self.dragging_filter = selected.map(|index| (index, self.snapshot.edit_revision));
        self.selected_filter = selected;
        cx.notify();
    }

    fn drag_graph_filter(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((index, revision)) = self.dragging_filter else {
            return;
        };
        if revision != self.snapshot.edit_revision {
            self.cancel_drag();
            cx.notify();
            return;
        }
        let Some(filter) = self.snapshot.equalizer.filters().get(index).copied() else {
            self.cancel_drag();
            return;
        };
        let (frequency_hz, gain_db) =
            parameters_at_position(self.graph_bounds.get(), event.position);
        self.pending_drag_preview = Some(DragPreview {
            index,
            filter: make_filter(
                filter.kind(),
                frequency_hz,
                gain_db,
                filter.quality_factor().get(),
            ),
            revision,
        });
        cx.notify();
    }

    fn end_graph_drag(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((index, revision)) = self.dragging_filter
            && revision == self.snapshot.edit_revision
            && let Some(filter) = self.snapshot.equalizer.filters().get(index).copied()
        {
            let (frequency_hz, gain_db) =
                parameters_at_position(self.graph_bounds.get(), event.position);
            self.pending_drag_preview = Some(DragPreview {
                index,
                filter: make_filter(
                    filter.kind(),
                    frequency_hz,
                    gain_db,
                    filter.quality_factor().get(),
                ),
                revision,
            });
        }
        self.dragging_filter = None;
        self.flush_pending_drag_preview(cx);
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

impl Render for TunicView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (status, status_color, route) =
            status_content(&self.snapshot.status, self.snapshot.bypassed);
        let button_label = if self.snapshot.bypassed {
            "Enable equalizer"
        } else {
            "Bypass equalizer"
        };
        let equalizer = self
            .pending_drag_preview
            .and_then(|preview| {
                preview.apply(&self.snapshot.equalizer, self.snapshot.edit_revision)
            })
            .unwrap_or_else(|| self.snapshot.equalizer.clone());
        let filters = equalizer.filters().to_vec();
        let sample_rate_hz = self
            .snapshot
            .active_route()
            .map_or(48_000.0, |route| route.sample_rate_hz);
        let response = PreparedGraph::prepare(&equalizer, sample_rate_hz).ok();
        let graph_bounds = Rc::clone(&self.graph_bounds);
        let has_unsaved_changes = self.snapshot.has_unsaved_changes;

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_6()
            .bg(rgb(0x17191d))
            .text_color(rgb(0xe7e8ea))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child("Tunic EQ"),
                            )
                            .child(div().text_sm().text_color(status_color).child(status))
                            .child(div().text_sm().text_color(rgb(0x9a9da5)).child(route)),
                    )
                    .child(
                        div()
                            .id("toggle-bypass")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x30343b))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x404650)))
                            .child(button_label)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_bypass(cx))),
                    ),
            )
            .child(
                div()
                    .id("eq-graph")
                    .relative()
                    .h(px(330.0))
                    .w_full()
                    .rounded_md()
                    .overflow_hidden()
                    .bg(rgb(0x0d1117))
                    .border_1()
                    .border_color(rgb(0x3a404a))
                    .child(self.spectrum.clone())
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                graph_bounds.set(bounds);
                                let mut grid = PathBuilder::stroke(px(1.0));
                                for column in 0..=10 {
                                    let x =
                                        bounds.left() + bounds.size.width * (column as f32 / 10.0);
                                    grid.move_to(point(x, bounds.top()));
                                    grid.line_to(point(x, bounds.bottom()));
                                }
                                for row in 0..=8 {
                                    let y = bounds.top() + bounds.size.height * (row as f32 / 8.0);
                                    grid.move_to(point(bounds.left(), y));
                                    grid.line_to(point(bounds.right(), y));
                                }

                                let mut curve = PathBuilder::stroke(px(2.0));
                                for sample in 0..=256 {
                                    let ratio = sample as f64 / 256.0;
                                    let frequency_hz = frequency_at_ratio(ratio);
                                    let gain_db = response
                                        .as_ref()
                                        .map_or(0.0, |graph| graph.response_db_at(frequency_hz))
                                        .clamp(MIN_GAIN_DB, MAX_GAIN_DB);
                                    let x = bounds.left() + bounds.size.width * ratio as f32;
                                    let y = bounds.top()
                                        + bounds.size.height * gain_to_ratio(gain_db) as f32;
                                    if sample == 0 {
                                        curve.move_to(point(x, y));
                                    } else {
                                        curve.line_to(point(x, y));
                                    }
                                }
                                (grid.build().ok(), curve.build().ok())
                            },
                            |_, (grid, curve), window, _| {
                                if let Some(grid) = grid {
                                    window.paint_path(grid, rgba(0x89909b2c));
                                }
                                if let Some(curve) = curve {
                                    window.paint_path(curve, rgb(0x4fc3f7));
                                }
                            },
                        )
                        .size_full(),
                    )
                    .children(filters.iter().enumerate().map(|(index, filter)| {
                        let x = frequency_to_ratio(filter.frequency().get()) as f32;
                        let y = gain_to_ratio(filter.gain().get()) as f32;
                        let selected = self.selected_filter == Some(index);
                        div()
                            .absolute()
                            .left(relative(x))
                            .top(relative(y))
                            .ml(px(-11.0))
                            .mt(px(-11.0))
                            .size(px(22.0))
                            .rounded_full()
                            .border_2()
                            .border_color(if selected {
                                rgb(0xffffff)
                            } else {
                                rgb(0x101318)
                            })
                            .bg(filter_color(index))
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child((index + 1).to_string())
                    }))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_graph_drag))
                    .on_mouse_move(cx.listener(Self::drag_graph_filter))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::end_graph_drag))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::end_graph_drag))
                    .child(
                        div()
                            .absolute()
                            .left(px(8.0))
                            .top(px(8.0))
                            .text_xs()
                            .text_color(rgb(0x8e949e))
                            .child("+24 dB"),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(8.0))
                            .bottom(px(8.0))
                            .text_xs()
                            .text_color(rgb(0x8e949e))
                            .child("−24 dB   20 Hz"),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(px(8.0))
                            .bottom(px(8.0))
                            .text_xs()
                            .text_color(rgb(0x8e949e))
                            .child("20 kHz"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .id("add-filter")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .bg(rgb(0x2e6f9e))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(0x3888bf)))
                                    .child("+ Add filter")
                                    .on_click(cx.listener(|this, _, _, cx| this.add_filter(cx))),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x9a9da5))
                                    .child("Drag a numbered point to change frequency and gain"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("revert-eq")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .bg(if has_unsaved_changes {
                                        rgb(0x3b3f47)
                                    } else {
                                        rgb(0x25282e)
                                    })
                                    .text_color(if has_unsaved_changes {
                                        rgb(0xe7e8ea)
                                    } else {
                                        rgb(0x686d76)
                                    })
                                    .cursor_pointer()
                                    .child("Revert")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.snapshot.has_unsaved_changes {
                                            this.revert(cx);
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .id("save-eq")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .bg(if has_unsaved_changes {
                                        rgb(0x087443)
                                    } else {
                                        rgb(0x254238)
                                    })
                                    .text_color(if has_unsaved_changes {
                                        rgb(0xffffff)
                                    } else {
                                        rgb(0x739184)
                                    })
                                    .cursor_pointer()
                                    .child(if has_unsaved_changes { "Save" } else { "Saved" })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.snapshot.has_unsaved_changes {
                                            this.save(cx);
                                        }
                                    })),
                            ),
                    ),
            )
            .when_some(self.editor_error.clone(), |element, error| {
                element.child(div().text_sm().text_color(rgb(0xff7b72)).child(error))
            })
            .child(
                div()
                    .id("filter-list")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(filters.into_iter().enumerate().map(|(index, filter)| {
                        let selected = self.selected_filter == Some(index);
                        div()
                            .id(("filter-row", index))
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(if selected {
                                rgb(0x2c3139)
                            } else {
                                rgb(0x22252b)
                            })
                            .border_1()
                            .border_color(if selected {
                                filter_color(index)
                            } else {
                                rgb(0x343941)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected_filter = Some(index);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .size(px(22.0))
                                    .rounded_full()
                                    .bg(filter_color(index))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .font_weight(FontWeight::BOLD)
                                    .child((index + 1).to_string()),
                            )
                            .child(
                                div()
                                    .id(("filter-kind", index))
                                    .w(px(92.0))
                                    .px_2()
                                    .py_1()
                                    .rounded_sm()
                                    .bg(rgb(0x383d46))
                                    .cursor_pointer()
                                    .child(filter_kind_label(filter.kind()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.cycle_filter_kind(index, cx);
                                        cx.stop_propagation();
                                    })),
                            )
                            .child(parameter_control(
                                "Hz",
                                format_frequency(filter.frequency().get()),
                                index,
                                "frequency",
                                cx,
                                |this, index, direction, cx| {
                                    this.adjust_filter(
                                        index,
                                        if direction < 0 { 0.8 } else { 1.25 },
                                        0.0,
                                        1.0,
                                        cx,
                                    );
                                },
                            ))
                            .child(parameter_control(
                                "dB",
                                format!("{:+.1}", filter.gain().get()),
                                index,
                                "gain",
                                cx,
                                |this, index, direction, cx| {
                                    this.adjust_filter(index, 1.0, f64::from(direction), 1.0, cx);
                                },
                            ))
                            .child(parameter_control(
                                "Q",
                                format!("{:.2}", filter.quality_factor().get()),
                                index,
                                "q",
                                cx,
                                |this, index, direction, cx| {
                                    this.adjust_filter(
                                        index,
                                        1.0,
                                        0.0,
                                        if direction < 0 { 0.8 } else { 1.25 },
                                        cx,
                                    );
                                },
                            ))
                            .child(
                                div()
                                    .id(("remove-filter", index))
                                    .ml_auto()
                                    .px_2()
                                    .py_1()
                                    .rounded_sm()
                                    .bg(rgb(0x493035))
                                    .text_color(rgb(0xffa5a5))
                                    .cursor_pointer()
                                    .child("Remove")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_filter(index, cx);
                                        cx.stop_propagation();
                                    })),
                            )
                    }))
                    .when(self.snapshot.equalizer.filters().is_empty(), |element| {
                        element.child(
                            div()
                                .p_4()
                                .text_color(rgb(0x9a9da5))
                                .child("No filters. Add one to begin editing."),
                        )
                    }),
            )
    }
}

const MIN_FREQUENCY_HZ: f64 = 20.0;
const MAX_FREQUENCY_HZ: f64 = 20_000.0;
const MIN_GAIN_DB: f64 = -24.0;
const MAX_GAIN_DB: f64 = 24.0;
const MIN_Q: f64 = 0.1;
const MAX_Q: f64 = 10.0;
const SPECTRUM_FLOOR_DB: f32 = -72.0;

fn parameter_control<F>(
    label: &'static str,
    value: String,
    index: usize,
    id: &'static str,
    cx: &mut Context<TunicView>,
    adjust: F,
) -> impl IntoElement
where
    F: Fn(&mut TunicView, usize, i32, &mut Context<TunicView>) + Copy + 'static,
{
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .id(format!("{id}-{index}-less"))
                .size(px(24.0))
                .rounded_sm()
                .bg(rgb(0x383d46))
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .child("−")
                .on_click(cx.listener(move |this, _, _, cx| {
                    adjust(this, index, -1, cx);
                    cx.stop_propagation();
                })),
        )
        .child(
            div()
                .w(px(60.0))
                .text_center()
                .child(value)
                .child(div().text_xs().text_color(rgb(0x8e949e)).child(label)),
        )
        .child(
            div()
                .id(format!("{id}-{index}-more"))
                .size(px(24.0))
                .rounded_sm()
                .bg(rgb(0x383d46))
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .child("+")
                .on_click(cx.listener(move |this, _, _, cx| {
                    adjust(this, index, 1, cx);
                    cx.stop_propagation();
                })),
        )
}

fn make_filter(kind: FilterKind, frequency_hz: f64, gain_db: f64, q: f64) -> Filter {
    Filter::new(
        kind,
        FrequencyHz::new(frequency_hz).expect("editor frequency is clamped to a valid range"),
        GainDb::new(gain_db).expect("editor gain is clamped to a valid range"),
        QualityFactor::new(q).expect("editor Q is clamped to a valid range"),
    )
}

fn frequency_to_ratio(frequency_hz: f64) -> f64 {
    (frequency_hz / MIN_FREQUENCY_HZ).ln() / (MAX_FREQUENCY_HZ / MIN_FREQUENCY_HZ).ln()
}

fn frequency_at_ratio(ratio: f64) -> f64 {
    MIN_FREQUENCY_HZ * (MAX_FREQUENCY_HZ / MIN_FREQUENCY_HZ).powf(ratio)
}

fn gain_to_ratio(gain_db: f64) -> f64 {
    (MAX_GAIN_DB - gain_db) / (MAX_GAIN_DB - MIN_GAIN_DB)
}

fn gain_at_ratio(ratio: f64) -> f64 {
    MAX_GAIN_DB - ratio * (MAX_GAIN_DB - MIN_GAIN_DB)
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

fn filter_position(bounds: Bounds<Pixels>, filter: Filter) -> gpui::Point<Pixels> {
    point(
        bounds.left() + bounds.size.width * frequency_to_ratio(filter.frequency().get()) as f32,
        bounds.top() + bounds.size.height * gain_to_ratio(filter.gain().get()) as f32,
    )
}

fn parameters_at_position(bounds: Bounds<Pixels>, position: gpui::Point<Pixels>) -> (f64, f64) {
    let width = bounds.size.width.as_f32().max(1.0);
    let height = bounds.size.height.as_f32().max(1.0);
    let x_ratio = f64::from(((position.x - bounds.left()).as_f32() / width).clamp(0.0, 1.0));
    let y_ratio = f64::from(((position.y - bounds.top()).as_f32() / height).clamp(0.0, 1.0));
    (frequency_at_ratio(x_ratio), gain_at_ratio(y_ratio))
}

fn filter_kind_label(kind: FilterKind) -> &'static str {
    match kind {
        FilterKind::Peaking => "Bell",
        FilterKind::LowShelf => "Low shelf",
        FilterKind::HighShelf => "High shelf",
    }
}

fn format_frequency(frequency_hz: f64) -> String {
    if frequency_hz >= 1_000.0 {
        format!("{:.1}k", frequency_hz / 1_000.0)
    } else {
        format!("{frequency_hz:.0}")
    }
}

fn filter_color(index: usize) -> gpui::Rgba {
    const COLORS: [u32; 8] = [
        0xe06c75, 0xe5c07b, 0x98c379, 0x56b6c2, 0x61afef, 0xc678dd, 0xd19a66, 0x7fdbca,
    ];
    rgb(COLORS[index % COLORS.len()])
}

fn status_content(status: &EngineStatus, bypassed: bool) -> (String, gpui::Rgba, String) {
    match status {
        EngineStatus::Starting => (
            "Starting".into(),
            rgb(0x8a6100),
            "Waiting for output".into(),
        ),
        EngineStatus::Running(route) => (
            if bypassed {
                "Bypassed".into()
            } else {
                "Processing".into()
            },
            if bypassed {
                rgb(0x8a6100)
            } else {
                rgb(0x087443)
            },
            format!(
                "{} · {:.0} Hz · {} ch",
                route.device_name, route.sample_rate_hz, route.channels
            ),
        ),
        EngineStatus::Failed(error) => ("Failed".into(), rgb(0xb42318), error.clone()),
        EngineStatus::Stopped => ("Stopped".into(), rgb(0x666666), "No output".into()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::{Bounds, point, px, size};

    use super::{
        DragPreview, MAX_FREQUENCY_HZ, MAX_GAIN_DB, MIN_FREQUENCY_HZ, MIN_GAIN_DB,
        frequency_at_ratio, frequency_to_ratio, make_filter, parameters_at_position,
        spectrum_amplitude_to_ratio, status_content,
    };
    use tunic_dsp::{Equalizer, FilterKind};
    use tunic_engine::{
        ActiveRoute, AudioPlatform, BypassControl, DeviceId, Engine, EngineOptions, EngineStatus,
        OutputDevice, PlatformError, PlatformEventSink, PlatformState, ProcessedOutputSink,
        TelemetryPublisher,
    };

    #[test]
    fn running_status_distinguishes_processing_from_bypass() {
        let status = EngineStatus::Running(ActiveRoute {
            device_id: DeviceId::new("output"),
            device_name: "Speakers".into(),
            sample_rate_hz: 48_000.0,
            channels: 2,
        });

        assert_eq!(status_content(&status, false).0, "Processing");
        assert_eq!(status_content(&status, true).0, "Bypassed");
    }

    #[test]
    fn graph_projection_is_logarithmic_for_frequency_and_linear_for_gain() {
        let geometric_middle = (MIN_FREQUENCY_HZ * MAX_FREQUENCY_HZ).sqrt();
        assert!((frequency_to_ratio(geometric_middle) - 0.5).abs() < f64::EPSILON);
        assert!((frequency_at_ratio(0.5) - geometric_middle).abs() < 1e-12);

        let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(800.0), px(400.0)));
        let (left_frequency, top_gain) =
            parameters_at_position(bounds, point(px(-100.0), px(-100.0)));
        let (right_frequency, bottom_gain) =
            parameters_at_position(bounds, point(px(1_000.0), px(1_000.0)));

        assert_eq!(left_frequency, MIN_FREQUENCY_HZ);
        assert_eq!(right_frequency, MAX_FREQUENCY_HZ);
        assert_eq!(top_gain, MAX_GAIN_DB);
        assert_eq!(bottom_gain, MIN_GAIN_DB);
    }

    #[test]
    fn spectrum_projection_maps_dbfs_to_the_graph_height() {
        assert_eq!(spectrum_amplitude_to_ratio(1.0), 0.0);
        assert!((spectrum_amplitude_to_ratio(0.001) - 5.0 / 6.0).abs() < 1e-6);
        assert_eq!(spectrum_amplitude_to_ratio(0.0), 1.0);
        assert_eq!(spectrum_amplitude_to_ratio(2.0), 0.0);
    }

    #[test]
    fn pending_drag_does_not_apply_to_a_new_edit_revision() {
        let engine = Engine::start(EngineOptions::default(), || TestPlatform).unwrap();
        let original_revision = engine.snapshot().edit_revision;
        let replacement =
            Equalizer::with_filter(make_filter(FilterKind::Peaking, 2_000.0, 3.0, 1.0));
        let replacement_revision = engine
            .preview_equalizer(replacement.clone(), original_revision)
            .unwrap();
        let pending = DragPreview {
            index: 0,
            filter: make_filter(FilterKind::Peaking, 8_000.0, -6.0, 1.0),
            revision: original_revision,
        };

        assert!(pending.apply(&replacement, replacement_revision).is_none());

        engine.shutdown().unwrap();
    }

    struct TestPlatform;

    impl AudioPlatform for TestPlatform {
        fn start(
            &mut self,
            _events: PlatformEventSink,
            _output_sink: Option<Arc<dyn ProcessedOutputSink>>,
            _telemetry: TelemetryPublisher,
            _equalizer: &Equalizer,
            _bypass: BypassControl,
        ) -> Result<PlatformState, PlatformError> {
            let route = ActiveRoute {
                device_id: DeviceId::new("output"),
                device_name: "Speakers".into(),
                sample_rate_hz: 48_000.0,
                channels: 2,
            };
            Ok(PlatformState::new(
                route.clone(),
                vec![OutputDevice {
                    id: route.device_id.clone(),
                    name: route.device_name.clone(),
                    sample_rate_hz: route.sample_rate_hz,
                    channels: route.channels,
                    is_default: true,
                }],
            ))
        }

        fn rebuild_default_route(
            &mut self,
            _equalizer: &Equalizer,
        ) -> Result<PlatformState, PlatformError> {
            unreachable!("this test does not rebuild the route")
        }

        fn set_equalizer(&mut self, _equalizer: &Equalizer) -> Result<(), PlatformError> {
            Ok(())
        }

        fn shutdown(&mut self) -> Result<(), PlatformError> {
            Ok(())
        }
    }
}
