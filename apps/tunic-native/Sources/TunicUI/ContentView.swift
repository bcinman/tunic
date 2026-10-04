import SwiftUI

public struct ContentView: View {
    private let quit: () -> Void

    public init(quit: @escaping () -> Void = {}) {
        self.quit = quit
    }

    public var body: some View {
        VStack {
            CurrentDevice()
            VStack {}
                .frame(height: 315, alignment: .topLeading)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .frame(width: 320)

    }
}

#Preview {
    ContentView()
}


private struct CurrentDevice: View {
    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "headphones")
                .font(.system(size: 16, weight: .medium))
                .foregroundStyle(.secondary)

            Text("External Headphones")
                .font(.body)

            Spacer()

            Toggle("Output enabled", isOn: .constant(true))
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.mini)
                .tint(.green)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("External Headphones, output enabled")
    }
}

private struct Control: View {
    @State private var gain = 0.0

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Bass")
                Spacer()
                Image(systemName: "arrow.counterclockwise")
            }
            Slider(value: $gain, in: -10...10)
        }
    }
}
