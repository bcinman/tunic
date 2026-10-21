import AppKit
import SwiftUI
import TunicUI

@main
struct TunicApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        MenuBarExtra("Tunic", systemImage: "slider.horizontal.3") {
            ContentView(model: delegate.model, updates: delegate.updates) {
                NSApplication.shared.terminate(nil)
            }
        }
        .menuBarExtraStyle(.window)
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let model = TunicModel()
    let updates = AppUpdates()

    func applicationWillFinishLaunching(_ notification: Notification) {
        NSApplication.shared.appearance = NSAppearance(named: .darkAqua)
    }

    func applicationWillTerminate(_ notification: Notification) {
        model.shutdown()
    }
}
