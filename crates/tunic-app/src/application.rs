use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, AppContext, Bounds, Entity, KeyBinding, Menu, MenuItem, WindowBounds, WindowOptions,
    actions, px, size,
};
use tunic_engine::{Engine, EngineOptions};
use tunic_macos::CoreAudioPlatform;

use crate::editor::TunicView;
use crate::{AppConfig, AppError};

actions!(tunic, [CloseWindow, Quit, ShowWindow]);

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
