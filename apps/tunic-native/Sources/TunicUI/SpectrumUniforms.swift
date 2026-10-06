/// Fixed-size GPU payload matching FieldParameters in Spectrum.metal.
/// SIMD4 fields keep the layout explicit and avoid a heap array on every draw.
struct SpectrumUniforms {
    var viewport: SIMD4<Float>
    var mapping: SIMD4<Float>
    var halftone: SIMD4<Float>
    var variation: SIMD4<Float>
    var ripple: SIMD4<Float>
    var wave0: SIMD4<Float>
    var wave1: SIMD4<Float>
    var wave2: SIMD4<Float>
    var wave3: SIMD4<Float>

    init(size: SIMD2<Float>, scale: Float, count: Int, style: SpectrumStyle,
         chromaticShift: Float, ripples: SpectrumRipples) {
        viewport = SIMD4(size.x, size.y, scale, Float(count))
        mapping = SIMD4(Float(style.width), Float(style.spread), Float(style.hue), Float(style.shading.rawValue))
        halftone = SIMD4(Float(style.dotSize), Float(style.dotSpacing), Float(style.amplitudeResponse),
                        Float(style.dotPattern.rawValue))
        variation = SIMD4(Float(style.verticalResponse), chromaticShift, style.whiteHalftone ? 1 : 0, 0)
        ripple = SIMD4(style.rippleActive ? Float(style.rippleStrength) : 0, Float(style.rippleWidth), 0, 0)
        func wave(_ index: Int) -> SIMD4<Float> {
            guard index < ripples.waves.count else { return .zero }
            let wave = ripples.waves[index]
            return SIMD4(Float(wave.x), Float(wave.y), 0, 0)
        }
        wave0 = wave(0)
        wave1 = wave(1)
        wave2 = wave(2)
        wave3 = wave(3)
    }
}
