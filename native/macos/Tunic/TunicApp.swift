import SwiftUI
import Tunic

@main
struct TunicApp: App {
    var body: some Scene {
        Window("Tunic", id: "main") {
            ContentView()
        }
        .windowStyle(.hiddenTitleBar)
        .windowToolbarStyle(.unified)
        .windowResizability(.contentSize)
        .defaultSize(width: 748, height: 320)
    }
}
