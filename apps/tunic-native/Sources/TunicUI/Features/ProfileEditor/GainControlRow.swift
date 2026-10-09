import SwiftUI
import TunicEngine

struct GainControlRow: View {
    let control: Control
    let onGainChange: (Double) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(control.name).lineLimit(1)
                Spacer()
                Text("\(control.gain > 0 ? "+" : "")\(control.gain.formatted(.number.precision(.fractionLength(0...1)))) dB")
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
            .font(.system(size: 12))
            Slider(value: Binding(get: { control.gain }, set: { onGainChange($0) }),
                    in: -12...12, neutralValue: 0) {
                Text(control.name)
            }
            .labelsHidden()
            .accessibilityValue("\(control.gain.formatted()) decibels")
        }.padding(.horizontal, 16)
    }
}
