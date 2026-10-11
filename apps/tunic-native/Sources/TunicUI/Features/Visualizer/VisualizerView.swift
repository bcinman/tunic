import SwiftUI

struct VisualizerView: View {
    let model: TunicModel
    let maximumFrequency: Double
    var hoveredFilter: UInt32?
    @State private var style = SpectrumStyle()
    @State private var demo = false
    @State private var labPresented = false
    @State private var hovered = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 0) {
            spectrum
            VisualizerSettings(style: $style, demo: $demo, isExpanded: labPresented)
        }
    }

    private var spectrum: some View {
        let influence = hoveredFilter.flatMap { model.gainInfluences[$0] }
        return LiveSpectrum(model: model, maximumFrequency: maximumFrequency,
                     style: style, demo: demo, hoverChanged: { hovered = $0 })
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
            .overlay(alignment: .topTrailing) {
                SpectrumLabButton(isPresented: $labPresented)
                    .animation(reduceMotion ? nil : .spring(VisualizerSettings.disclosureSpring), value: labPresented)
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

// Keep per-frame disclosure layout updates out of the spectrum's observation boundary.
private struct VisualizerSettings: View {
    @Binding var style: SpectrumStyle
    @Binding var demo: Bool
    let isExpanded: Bool
    @State private var settingsHeight = 0.0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    // Opening: spring the viewport downward; fade in the stationary controls independently.
    // Closing: fade controls out quickly while the viewport and chevron spring closed.
    static let disclosureSpring = Spring(duration: 0.38, bounce: 0)
    private let contentReveal = Animation.easeOut(duration: 0.22).delay(0.06)
    private let contentDismiss = Animation.easeOut(duration: 0.10)
    private let expandedHeight = 260.0

    var body: some View {
        ScrollView {
            SpectrumDebugPanel(style: $style, demo: $demo)
                .padding(12)
        }
        .frame(height: expandedHeight)
        .opacity(isExpanded ? 1 : 0)
        .animation(reduceMotion ? nil : isExpanded ? contentReveal : contentDismiss,
                   value: isExpanded)
        .frame(height: settingsHeight, alignment: .top)
        .clipped()
        .background(.primary.opacity(0.035))
        .allowsHitTesting(isExpanded)
        .accessibilityHidden(!isExpanded)
        .task(id: [isExpanded, reduceMotion]) { await resizeSettings() }
    }

    private func resizeSettings() async {
        let target = isExpanded ? expandedHeight : 0
        let start = settingsHeight
        guard start != target else { return }
        let began = ProcessInfo.processInfo.systemUptime
        // Publish layout sizes, not just presentation frames: MenuBarExtra and sibling
        // controls must measure every intermediate height rather than jump to the target.
        while !Task.isCancelled {
            let elapsed = ProcessInfo.processInfo.systemUptime - began
            let finished = reduceMotion || elapsed >= Self.disclosureSpring.settlingDuration
            settingsHeight = finished ? target
                : start + Self.disclosureSpring.value(target: target - start, time: elapsed)
            if finished { return }
            do {
                try await Task.sleep(for: .milliseconds(16))
            } catch {
                return
            }
        }
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
