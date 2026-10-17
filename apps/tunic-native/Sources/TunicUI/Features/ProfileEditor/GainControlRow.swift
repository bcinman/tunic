import SwiftUI
import TunicEngine

struct GainControlRow: View {
    private enum Interaction {
        case idle
        case dragging(Double)
        case awaitingFinish(value: Double, receipt: UInt64)
        case cancelled
    }

    let profile: String
    let control: Control
    let processedCommand: UInt64
    let send: (EngineCommand) -> UInt64?
    var dragChanged: (Bool) -> Void = { _ in }
    @State private var interaction = Interaction.idle

    private var gain: Double {
        switch interaction {
        case .dragging(let value), .awaitingFinish(let value, _): value
        case .idle, .cancelled: control.gain
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(control.name.capitalized).lineLimit(1)
                Spacer()
                Text("\(gain > 0 ? "+" : "")\(gain.formatted(.number.precision(.fractionLength(0...1)))) dB")
                    .font(.system(size: 11))
                    .monospacedDigit()
                    .foregroundStyle(.opacity(0.5))
            }
            .font(.system(size: 12))
            Slider(value: Binding(get: { gain }, set: changeGain),
                   in: -12...12, neutralValue: 0, label: { Text(control.name.capitalized) },
                   onEditingChanged: editingChanged)
            .labelsHidden()
            .accessibilityValue("\(gain.formatted()) decibels")
        }.padding(.horizontal, 16)
        .onChange(of: processedCommand) {
            guard case .awaitingFinish(_, let receipt) = interaction,
                  processedCommand >= receipt else { return }
            interaction = .idle
        }
        .onExitCommand {
            guard case .dragging = interaction else { return }
            interaction = .cancelled
            dragChanged(false)
            _ = send(.cancelControlGain(profile: profile))
        }
        .onDisappear { editingChanged(false) }
    }

    private func changeGain(_ value: Double) {
        if case .cancelled = interaction { return }
        let accepted = send(.previewControlGain(profile: profile, filter: control.filter, gain: value)) != nil
        if case .dragging = interaction {
            interaction = .dragging(accepted ? value : control.gain)
        } else if accepted {
            // Keyboard and accessibility changes may arrive without a drag lifecycle.
            finish(value)
        } else {
            interaction = .idle
        }
    }

    private func editingChanged(_ editing: Bool) {
        if editing {
            interaction = .dragging(gain)
            dragChanged(true)
        } else if case .dragging(let value) = interaction {
            finish(value)
            dragChanged(false)
        } else if case .cancelled = interaction {
            interaction = .idle
        }
    }

    private func finish(_ value: Double) {
        if let receipt = send(.finishControlGain(profile: profile)) {
            interaction = .awaitingFinish(value: value, receipt: receipt)
        } else {
            interaction = .idle
        }
    }
}
