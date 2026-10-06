import Foundation
import Metal
import Testing
@testable import TunicUI

@Test
func shapeSmoothingMatchesGaussianAndPreservesGeometry() {
    let original: [SIMD2<Float>] = (0..<256).map { index in
        SIMD2(Float(index) / 127.5 - 1, index == 32 || index == 224 ? 1 : -1)
    }
    var shape = SpectrumShapeSmoothing()
    var points = original
    shape.apply(to: &points, amount: 0)
    #expect(points == original)
    shape.apply(to: &points, amount: 0.5)
    #expect(points.map(\.x) == original.map(\.x))
    #expect(points[32].y < points[224].y) // Bass impulse spreads much wider.
    #expect(points[29].y > -1)
    #expect(points[221].y == -1)
    for index in [0, 29, 32, 34, 221, 224, 255] {
        let sigma = 0.5 * (0.5 + 7.5 * pow(1 - Double(index) / 255, 2))
        let radius = Int(ceil(3 * sigma))
        var sum = 0.0
        var total = 0.0
        for offset in -radius...radius {
            let weight = exp(-Double(offset * offset) / (2 * sigma * sigma))
            sum += weight * Double(original[min(255, max(0, index + offset))].y)
            total += weight
        }
        #expect(abs(Double(points[index].y) - sum / total) < 0.00001)
    }
    var cropped = original.map { SIMD2(($0.x + 1) * 1.7 - 1, $0.y) }
    shape.apply(to: &cropped, amount: 0.5)
    #expect(cropped.map(\.y) == points.map(\.y))
    for level: Float in [-1, 0.25, 1] {
        var flat = original.map { SIMD2($0.x, level) }
        shape.apply(to: &flat, amount: 1)
        #expect(flat.allSatisfy { abs($0.y - level) < 0.000001 })
    }
    var envelope = SpectrumEnvelope()
    let bins = original.map { Float(pow(10, Double($0.y - 1) * 45 / 20)) }
    envelope.observe(bins, maximumFrequency: 20_000, at: 0, attack: 0, decay: 0, smoothing: 0.5)
    #expect(zip(envelope.points, points).allSatisfy { abs($0.y - $1.y) < 0.00001 })
}

@Test
func spectrumUniformsMatchMetalLayout() {
    #expect(MemoryLayout<SpectrumUniforms>.stride == 9 * 16)
    #expect(MemoryLayout<SpectrumUniforms>.offset(of: \.ripple) == 4 * 16)
    #expect(MemoryLayout<SpectrumUniforms>.offset(of: \.wave0) == 5 * 16)
    #expect(MemoryLayout<SpectrumUniforms>.offset(of: \.wave3) == 8 * 16)
}

@Test
func envelopeCanResizeAndChangeFrequencyWithoutMetal() {
    var envelope = SpectrumEnvelope()
    envelope.observe([1, 0.001, 0], maximumFrequency: 20_000, at: 0, attack: 0, decay: 100)
    #expect(envelope.points[0] == SIMD2(-1, 1))
    #expect(abs(envelope.points[1].y + 1.0 / 3) < 0.00001)
    #expect(envelope.points[2] == SIMD2(1, -1))
    envelope.observe([0, 1], maximumFrequency: 200, at: 1, attack: 0, decay: 100)
    #expect(abs(envelope.points[1].x - 5) < 0.00001)
    envelope.observe([], maximumFrequency: 200, at: 2, attack: 0, decay: 100)
    #expect(envelope.points.isEmpty)
    #expect(!envelope.isAnimating)
}

/// Differential check against the original full-segment SDF. Jagged bins exercise
/// nearby valleys, steep slopes, both signs, cropped frequency axes, and radius limits.
@Test @MainActor
func boundedHalftoneClearanceMatchesFullDistance() async throws {
    let device = try #require(MTLCreateSystemDefaultDevice())
    let sourceURL = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Sources/TunicUI/Spectrum.metal")
    let source = try String(contentsOf: sourceURL, encoding: .utf8)
    let kernel = """
    kernel void compareClearance(const device float2 *points [[buffer(0)]],
                                 device float2 *results [[buffer(1)]], uint id [[thread_position_in_grid]]) {
        float2 p = float2((id % 320) + 0.37, (id / 320) + 0.23);
        float limit = 0.5 + float(id % 19) * 0.5;
        float reference = clamp(-spectrumDistance(p, points, 256, float2(320, 160)), 0.0f, limit);
        float bounded = spectrumClearance(p, limit, points, 256, float2(320, 160));
        results[id] = float2(reference, bounded);
    }
    """
    let library = try await device.makeLibrary(source: source + kernel, options: nil)
    let pipeline = try await device.makeComputePipelineState(function: #require(library.makeFunction(name: "compareClearance")))
    let queue = try #require(device.makeCommandQueue())
    let count = 320 * 160
    let output = try #require(device.makeBuffer(length: count * MemoryLayout<SIMD2<Float>>.stride, options: .storageModeShared))
    for frequencyScale: Float in [1, 1.7] {
        let points: [SIMD2<Float>] = (0..<256).map { index in
            SIMD2(2 * Float(index) / 255 * frequencyScale - 1,
                  Float((index * 73) % 251) / 125 - 1)
        }
        let command = try #require(queue.makeCommandBuffer())
        let encoder = try #require(command.makeComputeCommandEncoder())
        encoder.setComputePipelineState(pipeline)
        points.withUnsafeBytes { encoder.setBytes($0.baseAddress!, length: $0.count, index: 0) }
        encoder.setBuffer(output, offset: 0, index: 1)
        encoder.dispatchThreads(MTLSize(width: count, height: 1, depth: 1),
                                threadsPerThreadgroup: MTLSize(width: pipeline.threadExecutionWidth, height: 1, depth: 1))
        encoder.endEncoding()
        await withCheckedContinuation { continuation in
            command.addCompletedHandler { _ in continuation.resume() }
            command.commit()
        }
        #expect(command.status == .completed)
        let results = output.contents().bindMemory(to: SIMD2<Float>.self, capacity: count)
        var maximumError: Float = 0
        var nearBoundary = 0
        for index in 0..<count {
            let result = results[index]
            maximumError = max(maximumError, abs(result.x - result.y))
            if result.x > 0 && result.x < 0.5 { nearBoundary += 1 }
        }
        #expect(nearBoundary > 100)
        #expect(maximumError < 0.0001)
    }
}
