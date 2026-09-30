use gpui::{
    App, AppContext, Bounds, Context, Entity, IntoElement, Render, Window, WindowBounds,
    WindowOptions, px, size,
};
use tunic_core::Chain;
use tunic_macos::AudioSession;
use tunic_ui::TunicView;

struct Root {
    view: Entity<TunicView>,
    _audio: Option<AudioSession>,
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.view.clone()
    }
}

fn main() {
    let (audio, controller, device, audio_error) = match AudioSession::start(Chain::default()) {
        Ok((session, controller)) => {
            let device = session.device_name().to_owned();
            (Some(session), Some(controller), device, None)
        }
        Err(error) => (
            None,
            None,
            "System Output (audio unavailable)".into(),
            Some(error.to_string()),
        ),
    };

    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(560.0), px(360.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                let view = cx.new(|_| TunicView::new(device, controller, audio_error));
                cx.new(|_| Root {
                    view,
                    _audio: audio,
                })
            },
        )
        .expect("failed to open Tunic window");
        cx.activate(true);
    });
}
