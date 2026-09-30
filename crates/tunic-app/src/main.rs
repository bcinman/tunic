mod apple;
mod audio;

use std::future::poll_fn;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::Duration;

use gpui::{
    App, AppContext, Bounds, Context, Entity, IntoElement, Render, Task, Window, WindowBounds,
    WindowOptions, px, size,
};
use tunic_core::Chain;
use tunic_ui::TunicView;

use crate::apple::Apple;
use crate::audio::{ChangeHandler, Connection, Platform};

const AUDIO_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const UNAVAILABLE_DEVICE: &str = "System Output (audio unavailable)";

struct Root<P: Platform> {
    view: Entity<TunicView>,
    platform: P,
    _event_task: Task<()>,
}

impl<P: Platform + 'static> Root<P> {
    fn new(mut platform: P, cx: &mut Context<Self>) -> Self {
        let changes = Arc::new(ChangeSignal::default());
        let notify: ChangeHandler = {
            let changes = Arc::clone(&changes);
            Arc::new(move || changes.notify())
        };
        let (connection, retry_initial) = match platform.watch_default_output(notify) {
            Ok(()) => {
                let connection = platform.refresh_default_output(&Chain::default());
                let retry = connection.is_err();
                (connection, retry)
            }
            Err(error) => (Err(error), false),
        };
        let (device, controller, error) = connection_state(connection);
        let view = cx.new(|_| TunicView::new(device, controller, error));
        if retry_initial {
            changes.notify();
        }
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
            platform,
            _event_task: event_task,
        }
    }

    fn refresh_default_output(&mut self, cx: &mut Context<Self>) -> bool {
        let chain = self.view.read(cx).active_chain();
        let update = self.platform.refresh_default_output(&chain);
        if matches!(update, Ok(None)) {
            return true;
        }
        let connected = update.is_ok();
        let (device, controller, error) = connection_state(update);
        self.view.update(cx, |view, cx| {
            view.replace_audio(device, controller, error);
            cx.notify();
        });
        connected
    }
}

impl<P: Platform + 'static> Render for Root<P> {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.view.clone()
    }
}

fn connection_state<E: std::fmt::Display>(
    result: Result<Option<Connection>, E>,
) -> (String, Option<tunic_core::Controller>, Option<String>) {
    match result {
        Ok(Some(connection)) => (connection.device_name, Some(connection.controller), None),
        Ok(None) => (UNAVAILABLE_DEVICE.into(), None, None),
        Err(error) => (UNAVAILABLE_DEVICE.into(), None, Some(error.to_string())),
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
    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(560.0), px(360.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| Root::new(Apple::default(), cx)),
        )
        .expect("failed to open Tunic window");
        cx.activate(true);
    });
}
