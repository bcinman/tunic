mod window;

use std::future::poll_fn;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::Duration;

use gpui::{
    App, AppContext, Bounds, Context, Entity, IntoElement, Render, Task, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, point, px, size,
};
use tunic_core::{ChangeHandler, Command, Platform, Session};
use tunic_presets::BundledCatalog;
use tunic_sqlite::SqlitePersistence;
use tunic_ui::TunicView;

use tunic_macos::MacosPlatform;

const AUDIO_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const MAX_WINDOW_WIDTH: f64 = 640.0;

struct Root {
    view: Entity<TunicView>,
    _event_task: Task<()>,
}

impl Root {
    fn new(session: Session, platform: impl Platform + 'static, cx: &mut Context<Self>) -> Self {
        let changes = Arc::new(ChangeSignal::default());
        let notify: ChangeHandler = {
            let changes = Arc::clone(&changes);
            Arc::new(move || changes.notify())
        };
        let session = session.with_audio(platform, notify);
        if session.audio_retry_needed() {
            changes.notify();
        }
        let view = cx.new(|cx| TunicView::new(session, cx));
        let event_task = cx.spawn(async move |this, cx| {
            loop {
                changes.changed().await;
                loop {
                    match this.update(cx, |this, cx| this.refresh_default_output(cx)) {
                        Ok(true) => break,
                        Ok(false) => {
                            cx.background_executor().timer(AUDIO_RETRY_INTERVAL).await;
                        }
                        Err(_) => return,
                    }
                }
            }
        });
        Self {
            view,
            _event_task: event_task,
        }
    }

    fn refresh_default_output(&mut self, cx: &mut Context<Self>) -> bool {
        self.view.update(cx, |view, cx| {
            view.execute(Command::RefreshAudio, cx);
            !view.session().audio_retry_needed()
        })
    }
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.view.clone()
    }
}

#[derive(Default)]
struct ChangeSignal {
    state: Mutex<ChangeState>,
}

#[derive(Default)]
struct ChangeState {
    pending: bool,
    waker: Option<Waker>,
}

impl ChangeSignal {
    fn notify(&self) {
        let waker = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.pending = true;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    async fn changed(&self) {
        poll_fn(|cx| {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.pending {
                state.pending = false;
                Poll::Ready(())
            } else {
                state.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await;
    }
}

fn main() {
    let session = match persisted_session() {
        Ok(session) => session,
        Err(error) => {
            eprintln!("Cannot open Tunic's saved state: {error}");
            std::process::exit(1);
        }
    };
    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(640.0), px(460.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_background: WindowBackgroundAppearance::Blurred,
                titlebar: Some(TitlebarOptions {
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(20.0), px(20.0))),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| {
                window::set_max_width(MAX_WINDOW_WIDTH);
                window::configure_backdrop_blur();
                cx.new(|cx| Root::new(session, MacosPlatform::default(), cx))
            },
        )
        .expect("failed to open Tunic window");
        cx.activate(true);
    });
}

fn persisted_session() -> Result<Session, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    let directory = std::path::PathBuf::from(home).join("Library/Application Support/Tunic");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let store = SqlitePersistence::open(directory.join("session.sqlite3"))
        .map_err(|error| format!("{error:?}"))?;
    Session::new(store, BundledCatalog).map_err(|error| format!("{error:?}"))
}
