import Foundation
import Metal
import Testing
@testable import TunicUI

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
