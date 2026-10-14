import SwiftUI

struct VisualizerView: View {
    let model: TunicModel
    let maximumFrequency: Double
    var hoveredFilter: UInt32?
    @State private var style = SpectrumStyle()
    @State private var demo = false
    @State private var labPresented = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 0) {
            spectrum
                .overlay(alignment: .bottomTrailing) {
                    labToggle
                        .padding(8)
                }
            ScrollView {
                VisualizationSettingsView(style: $style, demo: $demo)
                    .padding(.vertical, 12)
            }
            .frame(height: 260)
            .background(.white.opacity(0.1))
            // Retain the controls and scroll position without occupying space when collapsed.
            .frame(height: labPresented ? 260 : 0, alignment: .top)
            .clipped()
            .opacity(labPresented ? 1 : 0)
            .disabled(!labPresented)
            .allowsHitTesting(labPresented)
            .accessibilityHidden(!labPresented)
        }
    }

    private var labToggle: some View {
        Button {
            labPresented.toggle()
        } label: {
            Image(systemName: labPresented ? "chevron.up" : "chevron.down")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 28, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Visualizer settings")
        .accessibilityValue(labPresented ? "Expanded" : "Collapsed")
        .help(labPresented ? "Hide visualizer settings" : "Show visualizer settings")
    }

    private var spectrum: some View {
        let influence = hoveredFilter.flatMap { model.gainInfluences[$0] }
        return LiveSpectrum(model: model, maximumFrequency: maximumFrequency,
                            style: style, demo: demo)
            .frame(height: 140)
            .opacity(influence == nil ? 1 : 0.25)
            .overlay {
                if let influence {
                    GainResponseView(response: model.response, influence: influence)
                        .transition(.opacity)
                }
            }
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: hoveredFilter)
            .accessibilityLabel(demo ? "Demo spectrum" : "Live output spectrum")
    }
}

// Observe frames here so telemetry doesn't invalidate the feature's presentation state.
private struct LiveSpectrum: View {
    let model: TunicModel
    let maximumFrequency: Double
    let style: SpectrumStyle
    let demo: Bool

    var body: some View {
        SpectrumView(
            spectrum: demo ? demoSpectrum : model.telemetry?.spectrum ?? [],
            maximumFrequency: maximumFrequency,
            style: style
        )
    }
}
