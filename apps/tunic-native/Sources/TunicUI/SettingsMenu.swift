import ServiceManagement
import SwiftUI

struct SettingsMenu: View {
    let quit: () -> Void
    @State private var loginStatus = SMAppService.mainApp.status
    @State private var loginError: String?

    private var isAppBundle: Bool { Bundle.main.bundleURL.pathExtension == "app" }

    var body: some View {
        Menu {
            Toggle("Open at Login", isOn: Binding(
                get: { loginStatus == .enabled },
                set: setOpenAtLogin
            ))
            .disabled(!isAppBundle)
            .help(isAppBundle ? "Launch Tunic when you log in" : "Requires a signed Tunic app bundle")
            Divider()
            Button("Quit Tunic", action: quit)
                .keyboardShortcut("q")
        } label: {
            Image(systemName: "gearshape")
                .accessibilityLabel("Settings")
        }
        .menuIndicator(.hidden)
        .menuStyle(.borderlessButton)
        .fixedSize()
        .opacity(0.5)
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
