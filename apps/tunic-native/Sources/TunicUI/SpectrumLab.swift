import SwiftUI

struct SpectrumDebugPanel: View {
    @Binding var style: SpectrumStyle
    @Binding var demo: Bool
    @State private var expanded = true

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            VStack(alignment: .leading, spacing: 8) {
                Picker("Input", selection: $demo) {
                    Text("Live audio").tag(false)
                    Text("Demo").tag(true)
                }
                .pickerStyle(.segmented)
                Text("1 · Spectrum distance")
                    .foregroundStyle(.secondary)
                slider("Shape", value: $style.shapeSmoothing, range: 0...1, unit: "", enabled: true)
                    .help("Shape smoothing: rounds spectrum steps, strongest in the bass. 0: original shape; does not blur dots or delay bass hits")
                slider("Attack", value: $style.attack, range: 0...500, unit: "ms", enabled: true)
                    .help("Rise smoothing for all visualizer styles. 0: instant; time to cover 63% of the remaining distance")
                slider("Decay", value: $style.decay, range: 0...1500, unit: "ms", enabled: true)
                    .help("Fall smoothing for all visualizer styles. 0: instant; independent of chromatic decay")
                Picker("Geometry", selection: $style.mode) {
                    ForEach(SpectrumMode.allCases, id: \.self) { mode in
                        Text(mode.rawValue).tag(mode)
                    }
                }
                .pickerStyle(.segmented)
                Divider()
                Text("2 · Color mapping")
                    .foregroundStyle(.secondary)
                Picker("Shading", selection: $style.shading) {
                    ForEach(SpectrumShading.allCases, id: \.self) { shading in
                        Text(shading.label).tag(shading)
                    }
                }
                .disabled(style.mode == .points)
                if style.usesHalftone {
                    Picker("Pattern", selection: $style.dotPattern) {
                        ForEach(HalftonePattern.allCases, id: \.self) { pattern in
                            Text(pattern.label).tag(pattern)
                        }
                    }
                    .pickerStyle(.segmented)
                    slider("Dot size", value: $style.dotSize, range: 0.5...12, unit: "pt", enabled: true)
                        .help("Maximum dot diameter, capped at the spacing to keep dots distinct")
                    slider("Spacing", value: $style.dotSpacing, range: 3...18, unit: "pt", enabled: true)
                        .help("Distance between neighboring dot centers")
                    slider("Amplitude", value: $style.amplitudeResponse, range: 0...1, unit: "", enabled: true)
                        .help("0: uniform dots. 1: diameter follows spectrum height (−90…0 dBFS) at each dot's frequency")
                    slider("Vertical", value: $style.verticalResponse, range: -1...1, unit: "", enabled: true)
                        .help("−1: larger at the bottom. 0: no vertical change. +1: larger at the top of the local spectrum area")
                    HStack {
                        Text("−1: bottom larger")
                        Spacer()
                        Text("+1: top larger")
                    }
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                } else {
                    slider("Width", value: $style.width, range: 0.5...8, unit: "pt",
                           enabled: style.mode == .points || style.shading != .distance)
                    slider("Spread", value: $style.spread, range: 2...30, unit: "pt",
                           enabled: style.mode == .sdf && style.shading != .line)
                }
                if style.usesHalftone {
                    Picker("Color", selection: $style.whiteHalftone) {
                        Text("Hue").tag(false)
                        Text("White").tag(true)
                    }
                    .pickerStyle(.segmented)
                }
                slider("Hue", value: $style.hue, range: 0...1, unit: "",
                       enabled: style.mode == .sdf && style.shading != .distance
                           && !(style.shading == .halftone && style.whiteHalftone))
                if style.usesHalftone {
                    Divider()
                    Toggle("Bass ripples", isOn: $style.rippleEnabled)
                    if style.rippleEnabled {
                        slider("Strength", value: $style.rippleStrength, range: 0...30, unit: "pt", enabled: true)
                            .help("Vertical wave displacement of the halftone image, not dot size")
                        slider("Speed", value: $style.rippleSpeed, range: 50...600, unit: "pt/s", enabled: true)
                        slider("Width", value: $style.rippleWidth, range: 5...100, unit: "pt", enabled: true)
                        slider("Decay", value: $style.rippleDecay, range: 100...2000, unit: "ms", enabled: true)
                        Button("Preview ripple") { style.ripplePreview += 1 }
                            .help("Launch a ripple without waiting for a bass hit; works with Demo input")
                    }
                    if style.rippleEnabled || style.chromaticEnabled {
                        slider("Trigger", value: $style.bassThreshold, range: -60 ... -6, unit: "dB", enabled: true)
                            .help("Shared 20–180 Hz bass threshold for ripples and chromatic aberration")
                    }
                    Toggle("Bass chromatic aberration", isOn: $style.chromaticEnabled)
                    if style.chromaticEnabled {
                        slider("Split", value: $style.chromaticStrength, range: 0...16, unit: "pt", enabled: true)
                            .help("Maximum channel offset across the whole visualizer")
                        slider("Decay", value: $style.chromaticDecay, range: 50...1000, unit: "ms", enabled: true)
                    }
                }
            }
            .padding(.top, 8)
        } label: {
            HStack {
                Text("Visualizer lab").fontWeight(.medium)
                Spacer()
                Button("Reset") { style = SpectrumStyle() }
                    .help("Reset visual settings; audio settings are unchanged")
            }
        }
        .font(.caption)
        .controlSize(.mini)
        .padding(10)
        .background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8))
    }

    private func slider(_ label: String, value: Binding<Double>, range: ClosedRange<Double>,
                        unit: String, enabled: Bool) -> some View {
        HStack(spacing: 8) {
            Text(label).frame(width: 58, alignment: .leading)
            Slider(value: value, in: range)
                .accessibilityLabel(label)
            Text("\(value.wrappedValue, specifier: unit == "ms" || unit == "pt/s" ? "%.0f" : unit.isEmpty ? "%.2f" : "%.1f")\(unit)")
                .monospacedDigit()
                .lineLimit(1)
                .frame(width: 56, alignment: .trailing)
        }
        .disabled(!enabled)
        .opacity(enabled ? 1 : 0.4)
    }
}

/// A static input makes shader comparisons repeatable even without playing audio.
let demoSpectrum: [Float] = (0..<256).map { index in
    let x = Double(index) / 255
    let db = -80 + 60 * exp(-pow((x - 0.32) / 0.12, 2))
        + 35 * exp(-pow((x - 0.73) / 0.05, 2))
    return Float(pow(10, db / 20))
}
