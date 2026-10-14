import SwiftUI

struct VisualizerView: View {
    let model: TunicModel
    let maximumFrequency: Double
    var hoveredFilter: UInt32?
    @State private var style = SpectrumStyle()
    @State private var demo = false
    @State private var labPresented = false
    @State private var settingsHeight: CGFloat = 0
    @State private var spectrumHovered = false
    @FocusState private var toggleFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    // Hover: fade in, rise 10pt, and sharpen together; reverse on exit without bounce.
    private enum ToggleMotion {
        static let offset: CGFloat = 5
        static let blur: CGFloat = 10
        static let spring = Animation.spring(duration: 0.4, bounce: 0)
    }

    // Fade the settings independently of the surrounding layout movement.
    private enum SettingsMotion {
        static let spring = Animation.spring(duration: 0.4, bounce: 0)
        static let exitSpring = Animation.spring(duration: 0.2, bounce: 0)
        static let layoutSpring = Animation.spring(duration: 0.2, bounce: 0)
        static let layoutExitSpring = Animation.spring(duration: 0.4, bounce: 0)
    }

    private var toggleVisible: Bool { labPresented || spectrumHovered || toggleFocused }

    var body: some View {
        VStack(spacing: 0) {
            spectrum
                .overlay(alignment: .bottomTrailing) {
                    labToggle
                        .padding(.trailing, 16)
                }
                .contentShape(Rectangle())
                .onHover { spectrumHovered = $0 }
            VisualizationSettingsView(style: $style, demo: $demo)
                .padding(.vertical, 12)
                .fixedSize(horizontal: false, vertical: true)
                .background(.white.opacity(0.1))
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .padding(.horizontal, 8)
                .padding(.top, 8)
                .onGeometryChange(for: CGFloat.self) { proxy in
                    proxy.size.height
                } action: { height in
                    settingsHeight = height
                }
                .opacity(labPresented ? 1 : 0)
                .animation(reduceMotion ? .easeOut(duration: 0.15)
                           : (labPresented ? SettingsMotion.spring : SettingsMotion.exitSpring), value: labPresented)
                // Animate occupied space independently of the mounted panel's entrance.
                .frame(height: labPresented ? settingsHeight : 0, alignment: .top)
                .disabled(!labPresented)
                .allowsHitTesting(labPresented)
                .accessibilityHidden(!labPresented)
        }
    }

    private var labToggle: some View {
        Button {
            let layoutAnimation = labPresented ? SettingsMotion.layoutExitSpring : SettingsMotion.layoutSpring
            withAnimation(reduceMotion ? nil : layoutAnimation) {
                labPresented.toggle()
            }
        } label: {
            Image(systemName: labPresented ? "chevron.up" : "slider.horizontal.3")
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.primary)
                .frame(width: 28, height: 28)
                .background(.white.opacity(0.14), in: Circle())
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .focused($toggleFocused)
        .blur(radius: toggleVisible || reduceMotion ? 0 : ToggleMotion.blur)
        .offset(y: toggleVisible || reduceMotion ? 0 : ToggleMotion.offset)
        .opacity(toggleVisible ? 1 : 0)
        .animation(reduceMotion ? .easeOut(duration: 0.15) : ToggleMotion.spring, value: toggleVisible)
        .allowsHitTesting(toggleVisible)
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
