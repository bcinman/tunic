import SwiftUI
import TunicEngine

struct GainControlRow: View {
    let control: Control
    let onGainChange: (Double) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(control.name).lineLimit(1)
                Spacer()
                Text(control.gain, format: .number.precision(.fractionLength(1)))
                    .monospacedDigit()
                Text("dB").foregroundStyle(.secondary)
            }
            .font(.caption)
            Slider(value: Binding(get: { control.gain }, set: { onGainChange($0) }), in: -12...12)
                .accessibilityLabel(control.name)
        }
        .padding(.horizontal, 12)
    }
}
