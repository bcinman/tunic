use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use tunic_ui::TunicView;

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(560.0), px(360.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| TunicView::new()),
        )
        .expect("failed to open Tunic window");
        cx.activate(true);
    });
}
