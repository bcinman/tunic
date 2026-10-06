/// Fixed-size GPU payload matching FieldParameters in Spectrum.metal.
/// SIMD4 fields keep the layout explicit and avoid a heap array on every draw.
struct SpectrumUniforms {
    var viewport: SIMD4<Float>
    var mapping: SIMD4<Float>
    var halftone: SIMD4<Float>
    var variation: SIMD4<Float>

    init(size: SIMD2<Float>, scale: Float, count: Int, style: SpectrumStyle,
         chromaticShift: Float) {
        viewport = SIMD4(size.x, size.y, scale, Float(count))
        mapping = SIMD4(Float(style.width), Float(style.spread), Float(style.hue), Float(style.shading.rawValue))
        halftone = SIMD4(Float(style.dotSize), Float(style.dotSpacing), Float(style.amplitudeResponse),
                        Float(style.dotPattern.rawValue))
        variation = SIMD4(Float(style.verticalResponse), chromaticShift, style.whiteHalftone ? 1 : 0, 0)
    }
}
