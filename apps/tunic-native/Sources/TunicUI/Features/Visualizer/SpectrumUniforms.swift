import SwiftUI

/// Fixed-size GPU payload matching FieldParameters in Spectrum.metal.
/// SIMD4 fields keep the layout explicit and avoid a heap array on every draw.
struct SpectrumUniforms {
    var viewport: SIMD4<Float>
    var halftone: SIMD4<Float>
    var response: SIMD4<Float>
    var solidColor: SIMD4<Float>
    var gradientStart: SIMD4<Float>
    var gradientSecond: SIMD4<Float>
    var gradientThird: SIMD4<Float>
    var gradientEnd: SIMD4<Float>

    init(size: SIMD2<Float>, scale: Float, count: Int, style: SpectrumStyle) {
        viewport = SIMD4(size.x, size.y, scale, Float(count))
        halftone = SIMD4(Float(style.dotSize), Float(style.dotSpacing), Float(style.amplitudeResponse),
                        Float(style.cellShape.rawValue))
        response = SIMD4(Float(style.sizeResponse), Float(style.opacityResponse), style.solidFill ? 1 : 0, 0)
        func rgb(_ color: Color) -> SIMD4<Float> {
            let resolved = color.resolve(in: EnvironmentValues())
            return SIMD4(resolved.red, resolved.green, resolved.blue, 1)
        }
        solidColor = rgb(style.solidColor)
        gradientStart = rgb(style.gradientStart)
        gradientSecond = rgb(style.gradientSecond)
        gradientThird = rgb(style.gradientThird)
        gradientEnd = rgb(style.gradientEnd)
    }
}
