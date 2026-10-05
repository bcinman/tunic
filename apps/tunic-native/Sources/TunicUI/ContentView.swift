import SwiftUI

public struct ContentView: View {
    private let model: TunicModel
    private let quit: () -> Void

    public init(model: TunicModel, quit: @escaping () -> Void = {}) {
        self.model = model
        self.quit = quit
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                Image(systemName: "headphones")
                    .foregroundStyle(.secondary)
                Text(model.snapshot?.deviceName ?? "Connecting…")
                    .lineLimit(1)
                Spacer()
                Circle()
                    .fill(model.snapshot?.acceptedChain == nil ? Color.secondary : Color.green)
                    .frame(width: 6, height: 6)
                    .accessibilityLabel(model.snapshot?.acceptedChain == nil ? "Audio unavailable" : "Processing")
            }

            if let state = model.snapshot {
                Menu {
                    Button("Flat") { model.enqueue(.useFlat) }
                    ForEach(state.presets, id: \.id) { preset in
                        Button("\(preset.brand) \(preset.model)") {
                            model.enqueue(.usePreset(id: preset.id))
                        }
                    }
                    Divider()
                    Button("Clear selection") { model.enqueue(.clearSelection) }
                } label: {
                    HStack {
                        Text(state.profileName ?? "Choose a profile")
                        Spacer()
                    }
                }
                .accessibilityLabel("Profile")

                LiveResponseGraph(model: model, maximumFrequency: min(20_000, state.sampleRate * 0.499))
                    .frame(height: 75)
                    .accessibilityLabel("Equalizer frequency response and live output spectrum")

                ForEach(state.controls, id: \.filter) { control in
                    VStack(alignment: .leading, spacing: 2) {
                        HStack {
                            Text(control.name).lineLimit(1)
                            Spacer()
                            Text(control.gain, format: .number.precision(.fractionLength(1)))
                                .monospacedDigit()
                            Text("dB").foregroundStyle(.secondary)
                        }
                        .font(.caption)
                        Slider(value: Binding(
                            get: { control.gain },
                            set: { model.enqueue(.setControlGain(filter: control.filter, gain: $0)) }
                        ), in: -12...12)
                        .accessibilityLabel(control.name)
                    }
                }

                HStack {
                    LiveLevels(model: model)
                    Spacer()
                    Button("Reset") { model.enqueue(.resetDraft) }
                        .disabled(!state.hasDraft)
                    Button("Save") { model.enqueue(.saveDraft) }
                        .disabled(!state.hasDraft)
                }
                .controlSize(.small)
            }

            if let error = model.error ?? model.snapshot?.actionError {
                Text(error).font(.caption).foregroundStyle(.red)
            }
            if let error = model.snapshot?.audioError {
                Text(error).font(.caption).foregroundStyle(.orange)
            }
            Divider()
            Button("Quit Tunic", action: quit)
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 12)
        .frame(width: 320)
        .task { await model.measureWhileVisible() }
    }
}

#Preview {
    ContentView(model: TunicModel(connectAudio: false))
}

// Read telemetry inside these bodies so a frame doesn't invalidate the controls.
private struct LiveResponseGraph: View {
    let model: TunicModel
    let maximumFrequency: Double

    var body: some View {
        ResponseGraph(
            response: model.response,
            spectrum: model.telemetry?.spectrum ?? [],
            maximumFrequency: maximumFrequency
        )
    }
}

private struct LiveLevels: View {
    let model: TunicModel

    var body: some View {
        Text(model.levelText)
            .font(.caption2.monospacedDigit())
            .foregroundStyle(.secondary)
    }
}

struct ResponseGraph: View {
    let response: [Double]
    let spectrum: [Float]
    let maximumFrequency: Double

    var body: some View {
        Canvas { context, size in
            let bounds = CGRect(origin: .zero, size: size)
            context.clip(to: Path(bounds))
            context.fill(
                SpectrumShape(amplitudes: spectrum, maximumFrequency: maximumFrequency).path(in: bounds),
                with: .color(.primary.opacity(0.12))
            )
            var zero = Path()
            zero.move(to: CGPoint(x: 0, y: size.height / 2))
            zero.addLine(to: CGPoint(x: size.width, y: size.height / 2))
            context.stroke(zero, with: .color(.secondary.opacity(0.2)), lineWidth: 1)
            guard response.count > 1 else { return }
            var curve = Path()
            for (index, gain) in response.enumerated() {
                let point = CGPoint(
                    x: size.width * Double(index) / Double(response.count - 1),
                    y: size.height * (0.5 - min(18, max(-18, gain)) / 36)
                )
                if index == 0 { curve.move(to: point) } else { curve.addLine(to: point) }
            }
            context.stroke(curve, with: .color(.primary.opacity(0.8)), lineWidth: 1.5)
        }
    }
}

/// Raw bins span 20 Hz–20 kHz. Spectrum height uses −90…0 dBFS,
/// independently of the response curve's ±18 dB gain scale.
struct SpectrumShape: Shape {
    let amplitudes: [Float]
    let maximumFrequency: Double

    func path(in rect: CGRect) -> Path {
        guard amplitudes.count > 1 else { return Path() }
        let frequencyScale = log(1_000.0) / log(maximumFrequency / 20)
        var path = Path()
        path.move(to: CGPoint(x: rect.minX, y: rect.maxY))
        for (index, amplitude) in amplitudes.enumerated() {
            let db = 20 * log10(max(Double(amplitude), 0.000_001))
            let height = min(1, max(0, (db + 90) / 90))
            path.addLine(to: CGPoint(
                x: rect.minX + rect.width * Double(index) / Double(amplitudes.count - 1) * frequencyScale,
                y: rect.maxY - rect.height * height
            ))
        }
        path.addLine(to: CGPoint(x: rect.minX + rect.width * frequencyScale, y: rect.maxY))
        path.closeSubpath()
        return path
    }
}
