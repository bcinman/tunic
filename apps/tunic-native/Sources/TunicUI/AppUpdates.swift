import Combine
import Sparkle
import SwiftUI

@MainActor
public final class AppUpdates: ObservableObject {
    @Published private(set) var canCheckForUpdates = false
    private var controller: SPUStandardUpdaterController?
    private var observation: NSKeyValueObservation?

    public init() {
        // Only release bundles configured by the publishing script may update themselves.
        #if !DEBUG
        guard Bundle.main.bundleURL.pathExtension == "app",
              Bundle.main.object(forInfoDictionaryKey: "SUPublicEDKey") != nil else { return }
        let controller = SPUStandardUpdaterController(startingUpdater: true,
                                                     updaterDelegate: nil, userDriverDelegate: nil)
        self.controller = controller
        observation = controller.updater.observe(
            \.canCheckForUpdates, options: [.initial, .new]
        ) { [weak self] _, change in
            let available = change.newValue ?? false
            Task { @MainActor [weak self] in self?.canCheckForUpdates = available }
        }
        #endif
    }

    var isEnabled: Bool { controller != nil }

    var automaticallyChecksForUpdates: Bool {
        get { controller?.updater.automaticallyChecksForUpdates ?? false }
        set {
            objectWillChange.send()
            controller?.updater.automaticallyChecksForUpdates = newValue
        }
    }

    func checkForUpdates() { controller?.checkForUpdates(nil) }
}

struct UpdateSettings: View {
    @ObservedObject var updates: AppUpdates

    var body: some View {
        Button("Check for Updates…", action: updates.checkForUpdates)
            .disabled(!updates.canCheckForUpdates)
        Toggle("Automatically Check for Updates", isOn: $updates.automaticallyChecksForUpdates)
            .disabled(!updates.isEnabled)
    }
}
