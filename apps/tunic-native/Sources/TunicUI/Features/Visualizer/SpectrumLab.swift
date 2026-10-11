import SwiftUI

struct SpectrumLabButton: View {
    @Binding var style: SpectrumStyle
    @Binding var demo: Bool
    @Binding var isPresented: Bool

    var body: some View {
        Button {
            isPresented.toggle()
        } label: {
            Image(systemName: "slider.horizontal.3")
                .frame(width: 28, height: 28)
                .contentShape(.interaction, Circle())
        }
        .buttonStyle(.plain)
        .focusEffectDisabled()
        .glassEffect(.regular.interactive(), in: .circle)
        .accessibilityLabel("Visualizer lab")
        .help("Visualizer lab")
        .popover(isPresented: $isPresented, arrowEdge: .trailing) {
            ScrollView {
                SpectrumDebugPanel(style: $style, demo: $demo)
                    .padding(12)
            }
            .frame(width: 340, height: 540)
        }
    }
}

struct SpectrumDebugPanel: View {
    @Binding var style: SpectrumStyle
    @Binding var demo: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Visualizer lab").fontWeight(.medium)
                Spacer()
                Button("Reset") { style = SpectrumStyle() }
                    .help("Reset visual settings; audio settings are unchanged")
            }
            VStack(alignment: .leading, spacing: 8) {
                Picker("Input", selection: $demo) {
                    Text("Live audio").tag(false)
                    Text("Demo").tag(true)
                }
                .pickerStyle(.segmented)
                Text("1 · Spectrum distance")
                    .foregroundStyle(.secondary)
                slider("Smoothing", value: $style.shapeSmoothing, range: 0...1, unit: "", enabled: true)
                    .help("Shape smoothing: rounds spectrum steps, strongest in the bass. "
                          + "0: original shape; does not blur dots or delay bass hits")
                slider("Attack", value: $style.attack, range: 0...500, unit: "ms", enabled: true)
                    .help("Rise smoothing for all visualizer styles. "
                          + "0: instant; time to cover 63% of the remaining distance")
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
                    Picker("Cell shape", selection: $style.cellShape) {
                        ForEach(CellShape.allCases, id: \.self) { shape in
                            Text(shape.label).tag(shape)
                        }
                    }
                    .pickerStyle(.segmented)
                    Toggle("Invert · cells become holes", isOn: $style.invertedHalftone)
                        .help("Fill around cells; size controls hole size and opacity controls hole transparency")
                    slider("Max size", value: $style.dotSize, range: 0.5...12, unit: "pt", enabled: true)
                        .help("Circle diameter or square width; sizes above the spacing overlap")
                    slider("Spacing", value: $style.dotSpacing, range: 3...18, unit: "pt", enabled: true)
                        .help("Distance between neighboring cell centers")
                    slider("Size response", value: $style.sizeResponse, range: 0...1, unit: "", enabled: true)
                        .help("0: constant size. 1: smaller at the bottom, larger toward the curve. "
                              + "Cells always fit the boundary")
                    slider("Opacity response", value: $style.opacityResponse, range: 0...1, unit: "", enabled: true)
                        .help("0: fully opaque. 1: fainter at the bottom, stronger toward the curve")
                    slider("Level influence", value: $style.amplitudeResponse, range: 0...1, unit: "", enabled: true)
                        .help("Makes size and opacity responses weaker in shorter parts of the spectrum. "
                              + "0: position only. 1: also scale by displayed spectrum height (−90…0 dBFS)")
                } else {
                    slider("Width", value: $style.width, range: 0.5...8, unit: "pt",
                           enabled: style.mode == .points || style.shading != .distance)
                    slider("Spread", value: $style.spread, range: 2...30, unit: "pt",
                           enabled: style.mode == .sdf && style.shading != .line)
                }
                if style.usesHalftone {
                    Picker("Color", selection: $style.solidFill) {
                        Text("Gradient").tag(false)
                        Text("Solid").tag(true)
                    }
                    .pickerStyle(.segmented)
                    if style.solidFill {
                        ColorPicker("Cell color", selection: $style.solidColor, supportsOpacity: false)
                    } else {
                        HStack {
                            ColorPicker("0%", selection: $style.gradientStart, supportsOpacity: false)
                            ColorPicker("33%", selection: $style.gradientSecond, supportsOpacity: false)
                            ColorPicker("67%", selection: $style.gradientThird, supportsOpacity: false)
                            ColorPicker("100%", selection: $style.gradientEnd, supportsOpacity: false)
                        }
                        .help("Colors blend left to right across the frequency axis")
                    }
                } else {
                    slider("Hue", value: $style.hue, range: 0...1, unit: "",
                           enabled: style.mode == .sdf && style.shading != .distance)
                }
                if style.usesHalftone {
                    Divider()
                    Toggle("Bass chromatic aberration", isOn: $style.chromaticEnabled)
                    if style.chromaticEnabled {
                        slider("Trigger", value: $style.bassThreshold, range: -60 ... -6, unit: "dB", enabled: true)
                            .help("20–180 Hz bass threshold for chromatic aberration")
                        slider("Split", value: $style.chromaticStrength, range: 0...16, unit: "pt", enabled: true)
                            .help("Maximum channel offset across the whole visualizer")
                        slider("Decay", value: $style.chromaticDecay, range: 50...1000, unit: "ms", enabled: true)
                    }
                }
                Divider()
                Toggle("Deep glow · after chromatic", isOn: $style.deepGlowEnabled)
                if style.deepGlowEnabled {
                    slider("Exposure", value: $style.deepGlowStrength, range: 0...6, unit: "", enabled: true)
                        .help("Glow intensity, blended in linear light behind the sharp image")
                    slider("Radius", value: $style.deepGlowRadius, range: 8...80, unit: "pt", enabled: true)
                        .help("Five blur scales approximate inverse-square falloff from a tight core to a broad halo")
                }
            }
            .padding(.top, 8)
        }
        .font(.caption)
        .controlSize(.mini)
        .padding(10)
        .background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8))
    }

    private func slider(_ label: String, value: Binding<Double>, range: ClosedRange<Double>,
                        unit: String, enabled: Bool) -> some View {
        HStack(spacing: 8) {
            Text(label).frame(width: 92, alignment: .leading)
            Slider(value: value, in: range)
                .accessibilityLabel(label)
            Text("\(value.wrappedValue, specifier: unit == "ms" ? "%.0f" : unit.isEmpty ? "%.2f" : "%.1f")\(unit)")
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
