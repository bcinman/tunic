import SwiftUI

struct AudioStatusView: View {
    let deviceName: String?
    let isProcessing: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "headphones")
                .foregroundStyle(.secondary)
            Text(deviceName ?? "Connecting…")
                .lineLimit(1)
            Spacer()
            Circle()
                .fill(isProcessing ? Color.green : Color.secondary)
                .frame(width: 6, height: 6)
                .accessibilityLabel(isProcessing ? "Processing" : "Audio unavailable")
        }
        .padding(.horizontal, 12)
        .padding(.top, 12)
    }
}
