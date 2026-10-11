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
            DisclosureGroup("Visualizer settings", isExpanded: $labPresented) {
                ScrollView {
                    SpectrumDebugPanel(style: $style, demo: $demo)
                        .padding(12)
                }
                .frame(height: 260)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
    }

    private var spectrum: some View {
        let influence = hoveredFilter.flatMap { model.gainInfluences[$0] }
        return LiveSpectrum(model: model, maximumFrequency: maximumFrequency,
                            style: style, demo: demo)
            .frame(height: 140)
            .opacity(influence == nil ? 1 : 0.22)
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
