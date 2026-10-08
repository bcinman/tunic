import SwiftUI

struct VisualizerView: View {
    let model: TunicModel
    let maximumFrequency: Double
    @State private var style = SpectrumStyle()
    @State private var demo = false
    @State private var labPresented = false
    @State private var hovered = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        LiveSpectrum(model: model, maximumFrequency: maximumFrequency,
                     style: style, demo: demo, hoverChanged: { hovered = $0 })
            .frame(height: 140)
            .accessibilityLabel(demo ? "Demo spectrum" : "Live output spectrum")
            .overlay(alignment: .topTrailing) {
                SpectrumLabButton(style: $style, demo: $demo, isPresented: $labPresented)
                    .padding(.trailing, 12)
                    .padding(.top, 6)
                    .opacity(hovered || labPresented ? 1 : 0)
                    .allowsHitTesting(hovered || labPresented)
                    .animation(reduceMotion ? nil : .easeInOut(duration: 0.2),
                               value: hovered || labPresented)
            }
            .onDisappear { hovered = false }
    }
}

// Observe frames here so telemetry doesn't invalidate the feature's presentation state.
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
