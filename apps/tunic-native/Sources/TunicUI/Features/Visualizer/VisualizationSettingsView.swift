import SwiftUI

struct VisualizationSettingsView: View {
    @Binding var style: SpectrumStyle
    @Binding var demo: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Visualizer settings").fontWeight(.medium)
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
                slider("Smoothing", value: $style.shapeSmoothing, range: 0...1, unit: "")
                    .help("Shape smoothing: rounds spectrum steps, strongest in the bass. "
                          + "0: original shape; does not blur dots or delay bass hits")
                slider("Attack", value: $style.attack, range: 0...500, unit: "ms")
                    .help("Rise smoothing for the spectrum. "
                          + "0: instant; time to cover 63% of the remaining distance")
                slider("Decay", value: $style.decay, range: 0...1500, unit: "ms")
                    .help("Fall smoothing for the spectrum. 0: instant")
                Divider()
                Text("2 · Grid fill")
                    .foregroundStyle(.secondary)
                Picker("Cell shape", selection: $style.cellShape) {
                    ForEach(CellShape.allCases, id: \.self) { shape in
                        Text(shape.label).tag(shape)
                    }
                }
                .pickerStyle(.segmented)
                slider("Max size", value: $style.dotSize, range: 0.5...12, unit: "pt")
                    .help("Circle diameter or square width; sizes above the spacing overlap")
                slider("Spacing", value: $style.dotSpacing, range: 3...18, unit: "pt")
                    .help("Distance between neighboring cell centers")
                slider("Size response", value: $style.sizeResponse, range: 0...1, unit: "")
                    .help("0: constant size. 1: smaller at the bottom, larger toward the curve. "
                          + "Cells always fit the boundary")
                slider("Opacity response", value: $style.opacityResponse, range: 0...1, unit: "")
                    .help("0: fully opaque. 1: fainter at the bottom, stronger toward the curve")
                slider("Level influence", value: $style.amplitudeResponse, range: 0...1, unit: "")
                    .help("Makes size and opacity responses weaker in shorter parts of the spectrum. "
                          + "0: position only. 1: also scale by displayed spectrum height (−90…0 dBFS)")
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
            }
            .padding(.top, 8)
        }
        .font(.caption)
        .controlSize(.mini)
        .padding(10)
        .background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8))
    }

    private func slider(_ label: String, value: Binding<Double>, range: ClosedRange<Double>,
                        unit: String) -> some View {
        HStack(spacing: 8) {
            Text(label).frame(width: 92, alignment: .leading)
            Slider(value: value, in: range)
                .accessibilityLabel(label)
            Text("\(value.wrappedValue, specifier: unit == "ms" ? "%.0f" : unit.isEmpty ? "%.2f" : "%.1f")\(unit)")
                .monospacedDigit()
                .lineLimit(1)
                .frame(width: 56, alignment: .trailing)
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
