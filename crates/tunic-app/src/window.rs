use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;

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
