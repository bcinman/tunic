import SwiftUI

struct VisualizationSettingsView: View {
    @Binding var style: SpectrumStyle
    @Binding var demo: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            row {
                HStack {
                    Text("Visualizer settings").fontWeight(.semibold)
                    Spacer()
                    Button("Reset") { style = SpectrumStyle() }
                        .help("Reset visual settings; audio settings are unchanged")
                }
            }
            Divider()
            row {
                HStack {
                    Text("Input")
                    Spacer()
                    Picker("Input", selection: $demo) {
                        Text("Live audio").tag(false)
                        Text("Demo").tag(true)
                    }
                    .labelsHidden()
                    .fixedSize()
                    .frame(width: 116, alignment: .trailing)
                }
            }
            Divider()
            row {
                HStack {
                    Text("Cell shape")
                    Spacer()
                    Picker("Cell shape", selection: $style.cellShape) {
                        ForEach(CellShape.allCases, id: \.self) { shape in
                            Text(shape.label).tag(shape)
                        }
                    }
                    .labelsHidden()
                    .fixedSize()
                    .frame(width: 116, alignment: .trailing)
                }
            }
            Divider()
            slider("Max size", value: $style.dotSize, range: 0.5...12, unit: "pt")
                .help("Circle diameter or square width; sizes above the spacing overlap")
            Divider()
            slider("Spacing", value: $style.dotSpacing, range: 3...18, unit: "pt")
                .help("Distance between neighboring cell centers")
            Divider()
            slider("Size response", value: $style.sizeResponse, range: 0...1, unit: "")
                .help("0: constant size. 1: smaller at the bottom, larger toward the curve. "
                      + "Cells always fit the boundary")
            Divider()
            slider("Opacity response", value: $style.opacityResponse, range: 0...1, unit: "")
                .help("0: fully opaque. 1: fainter at the bottom, stronger toward the curve")
            Divider()
            slider("Level influence", value: $style.amplitudeResponse, range: 0...1, unit: "")
                .help("Makes size and opacity responses weaker in shorter parts of the spectrum. "
                      + "0: position only. 1: also scale by displayed spectrum height (−90…0 dBFS)")
            Divider()
            row {
                HStack {
                    Text("Color")
                    Spacer()
                    Picker("Color", selection: $style.solidFill) {
                        Text("Gradient").tag(false)
                        Text("Solid").tag(true)
                    }
                    .labelsHidden()
                    .fixedSize()
                    .frame(width: 116, alignment: .trailing)
                }
            }
            Divider()
            row {
                if style.solidFill {
                    HStack {
                        Text("Solid color")
                        Spacer()
                        ColorPicker("Solid color", selection: $style.solidColor, supportsOpacity: false)
                            .labelsHidden()
                    }
                } else {
                    HStack {
                        Text("Gradient")
                        Spacer()
                        HStack {
                            ColorPicker("0%", selection: $style.gradientStart, supportsOpacity: false)
                            ColorPicker("33%", selection: $style.gradientSecond, supportsOpacity: false)
                            ColorPicker("67%", selection: $style.gradientThird, supportsOpacity: false)
                            ColorPicker("100%", selection: $style.gradientEnd, supportsOpacity: false)
                        }
                        .labelsHidden()
                    }
                    .help("Colors blend left to right across the frequency axis")
                }
            }
            .controlSize(.mini)
        }
        .font(.caption)
        .controlSize(.small)
        .pickerStyle(.menu)
    }

    private func row<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        content()
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
    }

    private func slider(_ label: String, value: Binding<Double>, range: ClosedRange<Double>,
                        unit: String) -> some View {
        row {
            HStack(spacing: 8) {
                Text(label).frame(width: 92, alignment: .leading)
                Slider(value: value, in: range)
                    .accessibilityLabel(label)
                Text("\(value.wrappedValue, specifier: unit.isEmpty ? "%.2f" : "%.1f")\(unit)")
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .frame(width: 40, alignment: .trailing)
            }
        }
    }
}

/// A static input makes shader comparisons repeatable even without playing audio.
let demoSpectrum: [Float] = (0..<256).map { index in
    let x = Double(index) / 255
    let db = -80 + 60 * exp(-pow((x - 0.32) / 0.12, 2))
        + 35 * exp(-pow((x - 0.73) / 0.05, 2))
    return Float(pow(10, db / 20))
}
