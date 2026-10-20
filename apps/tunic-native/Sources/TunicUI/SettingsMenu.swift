import ServiceManagement
import SwiftUI

struct HeaderActions: View {
    @Binding var isProcessing: Bool
    var processingAvailable = true
    var nowPlaying: NowPlayingModel?
    var updates: AppUpdates?
    let quit: () -> Void

    var body: some View {
        SettingsMenu(isProcessing: $isProcessing, processingAvailable: processingAvailable,
                     nowPlaying: nowPlaying, updates: updates, quit: quit)
    }
}

private struct HeaderActionStyle: ButtonStyle {
    @State private var hovered = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(.primary)
            .opacity(hovered ? 1 : 0.6)
            .contentShape(Rectangle())
            .onHover { hovered = $0 }
    }
}

struct SettingsMenu: View {
    @Binding var isProcessing: Bool
    var processingAvailable = true
    var nowPlaying: NowPlayingModel?
    var updates: AppUpdates?
    let quit: () -> Void
    @State private var loginStatus = SMAppService.mainApp.status
    @State private var loginError: String?

    private var isAppBundle: Bool { Bundle.main.bundleURL.pathExtension == "app" }

    var body: some View {
        Menu {
            Toggle("Processing", isOn: $isProcessing)
                .disabled(!processingAvailable)
            if let nowPlaying { NowPlayingSettings(model: nowPlaying) }
            Divider()
            Toggle("Open at Login", isOn: Binding(
                get: { loginStatus == .enabled },
                set: setOpenAtLogin
            ))
            .disabled(!isAppBundle)
            .help(isAppBundle ? "Launch Tunic when you log in" : "Requires a signed Tunic app bundle")
            Divider()
            if let updates { UpdateSettings(updates: updates) }
            Button("Quit Tunic", action: quit)
                .keyboardShortcut("q")
        } label: {
            Image(systemName: "gearshape")
                .font(.system(size: 14))
                .frame(width: 24, height: 24)
                .accessibilityLabel("Settings")
        }
        .menuIndicator(.hidden)
        .buttonStyle(HeaderActionStyle())
        .fixedSize()
        .help("Settings")
        .onAppear { loginStatus = SMAppService.mainApp.status }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            loginStatus = SMAppService.mainApp.status
        }
        .alert("Couldn’t change Open at Login", isPresented: Binding(
            get: { loginError != nil },
            set: { if !$0 { loginError = nil } }
        )) {
            Button("OK", role: .cancel) { loginError = nil }
        } message: {
            Text(loginError ?? "")
        }
    }

    private func setOpenAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                if SMAppService.mainApp.status != .requiresApproval {
                    try SMAppService.mainApp.register()
                }
                if SMAppService.mainApp.status == .requiresApproval {
                    SMAppService.openSystemSettingsLoginItems()
                }
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            loginError = error.localizedDescription
        }
        loginStatus = SMAppService.mainApp.status
    }
}
