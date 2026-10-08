import SwiftUI

/// Fixed-size GPU payload matching FieldParameters in Spectrum.metal.
/// SIMD4 fields keep the layout explicit and avoid a heap array on every draw.
struct SpectrumUniforms {
    var viewport: SIMD4<Float>
    var mapping: SIMD4<Float>
    var halftone: SIMD4<Float>
    var variation: SIMD4<Float>
    var gradientStart: SIMD4<Float>
    var gradientSecond: SIMD4<Float>
    var gradientThird: SIMD4<Float>
    var gradientEnd: SIMD4<Float>

    init(size: SIMD2<Float>, scale: Float, count: Int, style: SpectrumStyle,
         chromaticShift: Float) {
        viewport = SIMD4(size.x, size.y, scale, Float(count))
        mapping = SIMD4(Float(style.width), Float(style.spread), Float(style.hue), Float(style.shading.rawValue))
        halftone = SIMD4(Float(style.dotSize), Float(style.dotSpacing), Float(style.amplitudeResponse),
                        Float(style.dotPattern.rawValue))
        variation = SIMD4(0, chromaticShift, style.whiteHalftone ? 1 : 0,
                         style.invertedHalftone ? 1 : 0)
        func rgb(_ color: Color) -> SIMD4<Float> {
            let resolved = color.resolve(in: EnvironmentValues())
            return SIMD4(resolved.red, resolved.green, resolved.blue, 1)
        }
        gradientStart = rgb(style.gradientStart)
        gradientSecond = rgb(style.gradientSecond)
        gradientThird = rgb(style.gradientThird)
        gradientEnd = rgb(style.gradientEnd)
    }
}
