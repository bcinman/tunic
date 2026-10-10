import SwiftUI

struct AudioStatusView: View {
    let deviceName: String?
    @Binding var isProcessing: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "headphones")
                .foregroundStyle(.secondary)
            Text(deviceName ?? "Connecting…")
                .lineLimit(1)
            Spacer()
            Toggle("Processing", isOn: $isProcessing)
                .labelsHidden()
                .toggleStyle(SwitchToggleStyle(tint: .green))
                .controlSize(.mini)
                .help(isProcessing ? "Turn processing off" : "Turn processing on")
        }
        .padding(.horizontal, 12)
        .padding(.top, 12)
    }
}
