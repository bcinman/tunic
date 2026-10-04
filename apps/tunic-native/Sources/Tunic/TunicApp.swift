import AppKit
import SwiftUI
import TunicUI

@main
struct TunicApp: App {
    var body: some Scene {
        MenuBarExtra("Tunic", systemImage: "slider.horizontal.3") {
            ContentView {
                NSApplication.shared.terminate(nil)
            }
        }
        .menuBarExtraStyle(.window)
    }
}
