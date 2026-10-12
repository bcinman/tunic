import Foundation

/// Clip-space coordinates: bins span 20 Hz–20 kHz logarithmically; height is −90…0 dBFS.
func spectrumPoints(_ amplitudes: [Float], maximumFrequency: Double) -> [SIMD2<Float>] {
    var points: [SIMD2<Float>] = []
    updateSpectrumPoints(amplitudes, maximumFrequency: maximumFrequency, into: &points)
    return points
}

/// Reuse the telemetry-sized allocation between updates.
private func updateSpectrumPoints(_ amplitudes: [Float], maximumFrequency: Double,
                                  into points: inout [SIMD2<Float>]) {
    guard amplitudes.count > 1 else {
        points.removeAll(keepingCapacity: true)
        return
    }
    if points.count != amplitudes.count {
        points = Array(repeating: .zero, count: amplitudes.count)
    }
    let frequencyScale = log(1_000.0) / log(maximumFrequency / 20)
    for (index, amplitude) in amplitudes.enumerated() {
        let db = 20 * log10(max(Double(amplitude), 0.000_001))
        let height = min(1, max(0, (db + 90) / 90))
        points[index] = SIMD2(
            Float(2 * Double(index) / Double(amplitudes.count - 1) * frequencyScale - 1),
            Float(2 * height - 1)
        )
    }
}

/// Displayed spectrum geometry, independent of the rendering backend.
struct SpectrumEnvelope {
    var points: [SIMD2<Float>] = []
    private var target: [SIMD2<Float>] = []
    private var shape = SpectrumShapeSmoothing()
    private var lastTime: Double?

    var isAnimating: Bool { !target.isEmpty && points != target }

    mutating func observe(_ spectrum: [Float], maximumFrequency: Double, at time: Double,
                          attack: Double, decay: Double, smoothing: Double = 0) {
        advance(to: time, attack: attack, decay: decay)
        updateSpectrumPoints(spectrum, maximumFrequency: maximumFrequency, into: &target)
        shape.apply(to: &target, amount: smoothing)
        if points.count != target.count {
            points = target.map { SIMD2($0.x, -1) }
        }
        advance(to: time, attack: attack, decay: decay)
    }

    /// Milliseconds are exponential time constants applied to displayed dB height.
    mutating func advance(to time: Double, attack: Double, decay: Double) {
        let elapsed = max(0, time - (lastTime ?? time))
        lastTime = time
        // Only two coefficients per frame, not one transcendental call per bin.
        let rise = Float(attack == 0 ? 1 : -expm1(-elapsed * 1000 / attack))
        let fall = Float(decay == 0 ? 1 : -expm1(-elapsed * 1000 / decay))
        for index in target.indices {
            let difference = target[index].y - points[index].y
            points[index].x = target[index].x
            points[index].y += difference * (difference > 0 ? rise : fall)
            if abs(target[index].y - points[index].y) < 0.0001 { points[index].y = target[index].y }
        }
    }
}

/// Gaussian blur of displayed dB height, wider toward bass. Reuses scratch storage
/// and runs only on input updates, before temporal smoothing and GPU upload.
struct SpectrumShapeSmoothing {
    private var heights: [Float] = []

    mutating func apply(to points: inout [SIMD2<Float>], amount: Double) {
        guard amount > 0, points.count > 1 else { return }
        if heights.count != points.count { heights = Array(repeating: 0, count: points.count) }
        for index in points.indices { heights[index] = points[index].y }
        let last = points.count - 1
        for index in points.indices {
            // Use original bin position, not cropped clip x: tuning the maximum
            // displayed frequency must not change smoothing at a given frequency.
            let bass = 1 - Float(index) / Float(last)
            let sigma = Float(amount) * (0.5 + 7.5 * bass * bass) * Float(last) / 255
            let radius = Int(ceil(3 * sigma))
            // Gaussian weights via a recurrence: one exponential per bin, rather
            // than per tap. Clamp edges so constant levels and silence stay flat.
            var ratio = exp(-0.5 / (sigma * sigma))
            let ratioStep = ratio * ratio
            var weight: Float = 1
            var total: Float = 1
            var sum = heights[index]
            for offset in 1...radius {
                weight *= ratio
                ratio *= ratioStep
                sum += weight * (heights[max(0, index - offset)] + heights[min(last, index + offset)])
                total += 2 * weight
            }
            points[index].y = sum / total
        }
    }
}
