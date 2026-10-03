import AppKit
import SwiftUI

@main
struct TunicApp: App {
    var body: some Scene {
        MenuBarExtra("Tunic", systemImage: "slider.horizontal.3") {
            Button("Quit Tunic") {
                NSApplication.shared.terminate(nil)
            }
            .keyboardShortcut("q")
        }
    }
}
