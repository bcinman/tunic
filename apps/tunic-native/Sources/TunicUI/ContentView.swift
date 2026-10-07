import SwiftUI

public struct ContentView: View {
    private let model: TunicModel
    private let quit: () -> Void
    @State private var spectrumStyle = SpectrumStyle()
    @State private var demoSpectrumEnabled = false
    @State private var visualizerLabPresented = false
    @State private var visualizerHovered = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

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
            .padding(.horizontal, 12)
            .padding(.top, 12)

            if let state = model.snapshot {
                LiveSpectrum(model: model, maximumFrequency: min(20_000, state.sampleRate * 0.499),
                             style: spectrumStyle, demo: demoSpectrumEnabled,
                             hoverChanged: { visualizerHovered = $0 })
                    .frame(height: 140)
                    .accessibilityLabel(demoSpectrumEnabled ? "Demo spectrum" : "Live output spectrum")
                    .overlay(alignment: .topTrailing) {
                        SpectrumLabButton(style: $spectrumStyle, demo: $demoSpectrumEnabled,
                                          isPresented: $visualizerLabPresented)
                            .padding(.trailing, 12)
                            .padding(.top, 6)
                            .opacity(visualizerHovered || visualizerLabPresented ? 1 : 0)
                            .allowsHitTesting(visualizerHovered || visualizerLabPresented)
                            .animation(reduceMotion ? nil : .easeInOut(duration: 0.2),
                                       value: visualizerHovered || visualizerLabPresented)
                    }
                    .onDisappear { visualizerHovered = false }

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
                    HStack(spacing: 8) {
                        Text(state.profileName ?? "Choose a profile")
                            .lineLimit(1)
                        Image(systemName: "chevron.down")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                            .accessibilityHidden(true)
                    }
                }
                .menuIndicator(.hidden)
                .buttonStyle(.plain)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .glassEffect(.regular.interactive())
                .accessibilityLabel("Profile")
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.horizontal, 12)

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
                    .padding(.horizontal, 12)
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
                .padding(.horizontal, 12)
            }

            if let error = model.error ?? model.snapshot?.actionError {
                Text(error).font(.caption).foregroundStyle(.red)
                    .padding(.horizontal, 12)
            }
            if let error = model.snapshot?.audioError {
                Text(error).font(.caption).foregroundStyle(.orange)
                    .padding(.horizontal, 12)
            }
            Divider()
                .padding(.horizontal, 12)
            Button("Quit Tunic", action: quit)
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 12)
                .padding(.bottom, 12)
        }
        .frame(width: 320)
        .task { await model.measureWhileVisible() }
    }
}

#Preview {
    ContentView(model: TunicModel(connectAudio: false))
}

// Read telemetry inside these bodies so a frame doesn't invalidate the controls.
private struct LiveSpectrum: View {
    let model: TunicModel
    let maximumFrequency: Double
    let style: SpectrumStyle
    let demo: Bool
    let hoverChanged: (Bool) -> Void

    var body: some View {
        SpectrumView(
            spectrum: demo ? demoSpectrum : model.telemetry?.spectrum ?? [],
            maximumFrequency: maximumFrequency,
            style: style,
            hoverChanged: hoverChanged
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
