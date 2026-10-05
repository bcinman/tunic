import AppKit
import SwiftUI
import TunicUI

@main
struct TunicApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        MenuBarExtra("Tunic", systemImage: "slider.horizontal.3") {
            ContentView(model: delegate.model) {
                NSApplication.shared.terminate(nil)
            }
        }
        .menuBarExtraStyle(.window)
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let model = TunicModel()

    func applicationWillTerminate(_ notification: Notification) {
        model.shutdown()
    }
}
