use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectView,
};

pub fn set_max_width(width: f64) {
    let main_thread =
        MainThreadMarker::new().expect("window configuration runs on the main thread");
    let application = NSApplication::sharedApplication(main_thread);
    let window = application
        .windows()
        .firstObject()
        .expect("Tunic window exists during root-view construction");
    let mut max_size = window.contentMaxSize();
    max_size.width = width;
    window.setContentMaxSize(max_size);
}

pub fn configure_backdrop_blur() {
    let main_thread =
        MainThreadMarker::new().expect("window configuration runs on the main thread");
    let application = NSApplication::sharedApplication(main_thread);
    let window = application
        .windows()
        .firstObject()
        .expect("Tunic window exists during root-view construction");
    let content_view = window
        .contentView()
        .expect("Tunic window has a content view");

    for view in content_view.subviews().iter() {
        if let Some(blur_view) = view.downcast_ref::<NSVisualEffectView>() {
            blur_view.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
            blur_view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            return;
        }
    }

    panic!("GPUI blurred window has a visual effect view");
}
