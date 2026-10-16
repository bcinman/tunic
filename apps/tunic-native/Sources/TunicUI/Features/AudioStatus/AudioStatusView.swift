import SwiftUI

struct AudioStatusView: View {
    let deviceName: String?
    @Binding var isProcessing: Bool

    var body: some View {
        HStack(spacing: 8) {
            Toggle("Processing", isOn: $isProcessing)
                .labelsHidden()
                .toggleStyle(SwitchToggleStyle(tint: .green))
                .controlSize(.mini)
                .help(isProcessing ? "Turn processing off" : "Turn processing on")
            if let deviceName {
                Label(deviceName, systemImage: "headphones")
                    .lineLimit(1)
            }
        }
    }
}
