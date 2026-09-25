//! GPUI application lifecycle and presentation, wiring `tunic-engine` to the
//! `tunic-macos` platform implementation.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, FontWeight, IntoElement, KeyBinding, Menu, MenuItem, Render, Task,
    Window, WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size,
};
use tunic_engine::{Engine, EngineHandle, EngineOptions, EngineSnapshot, EngineStatus};
use tunic_macos::CoreAudioPlatform;

actions!(tunic, [Quit]);

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

    gpui_platform::application().run(move |cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu {
            name: "Tunic".into(),
            items: vec![MenuItem::action("Quit Tunic", Quit)],
            disabled: false,
        }]);

        let bounds = Bounds::centered(None, size(px(480.), px(320.)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Tunic".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| TunicView::new(engine, cx)),
        );
        if let Err(error) = result {
            eprintln!("error: open window: {error}");
            std::process::exit(1);
        }
        cx.activate(true);
    });
    Ok(())
}

struct TunicView {
    engine: Option<EngineHandle>,
    snapshot: EngineSnapshot,
    _refresh_task: Task<()>,
}

impl TunicView {
    fn new(engine: EngineHandle, cx: &mut Context<Self>) -> Self {
        let snapshot = engine.snapshot();
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
            _refresh_task: refresh_task,
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

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .p_6()
            .bg(rgb(0xf5f5f5))
            .text_color(rgb(0x202020))
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::BOLD)
                    .child("Tunic"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .rounded_md()
                    .bg(rgb(0xffffff))
                    .border_1()
                    .border_color(rgb(0xd8d8d8))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child("Status")
                            .child(div().text_color(status_color).child(status)),
                    )
                    .child(div().flex().justify_between().child("Output").child(route)),
            )
            .child(
                div()
                    .id("toggle-bypass")
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(0x333333))
                    .text_color(rgb(0xffffff))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x555555)))
                    .child(button_label)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_bypass(cx))),
            )
    }
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
    use super::status_content;
    use tunic_engine::{ActiveRoute, DeviceId, EngineStatus};

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
}
