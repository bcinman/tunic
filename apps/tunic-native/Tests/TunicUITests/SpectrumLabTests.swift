import AppKit
import MetalKit
import Observation
import SwiftUI
import Testing
@testable import TunicUI

@Test @MainActor
func spectrumAttackAndDecayUseElapsedTimeAndSettle() throws {
    let device = try #require(MTLCreateSystemDefaultDevice())
    let renderer = try SpectrumRenderer(device: device)
    renderer.style.attack = 100
    renderer.style.decay = 400
    renderer.receiveSpectrum([1, 0.001], maximumFrequency: 20_000, at: 0)
    renderer.advanceSpectrum(to: 0.1)
    #expect(abs(renderer.points[0].y - Float(1 - 2 / exp(Double(1)))) < 0.00001)
    #expect(abs(renderer.points[1].y - Float(-1 + (2.0 / 3) * (1 - 1 / exp(Double(1))))) < 0.00001)
    let split = try SpectrumRenderer(device: device)
    split.style = renderer.style
    split.receiveSpectrum([1, 0.001], maximumFrequency: 20_000, at: 0)
    for step in 1...10 { split.advanceSpectrum(to: Double(step) / 100) }
    #expect(abs(split.points[0].y - renderer.points[0].y) < 0.00001)
    let peak = renderer.points[0].y
    renderer.receiveSpectrum([0, 0], maximumFrequency: 20_000, at: 0.1)
    renderer.advanceSpectrum(to: 0.5)
    #expect(abs(renderer.points[0].y - (-1 + (peak + 1) / Float(exp(Double(1))))) < 0.00001)
    let view = MTKView(frame: .zero, device: device)
    renderer.updateAnimation(view)
    #expect(!view.isPaused)
    renderer.advanceSpectrum(to: 10)
    renderer.updateAnimation(view)
    #expect(view.isPaused)
    renderer.style.attack = 0
    renderer.receiveSpectrum([1, 1], maximumFrequency: 20_000, at: 10)
    #expect(renderer.points.map(\.y) == [1, 1])
    renderer.style.decay = 0
    renderer.receiveSpectrum([0, 0], maximumFrequency: 20_000, at: 10)
    #expect(renderer.points.map(\.y) == [-1, -1])
    renderer.style.attack = 500
    renderer.receiveSpectrum([1, 1], maximumFrequency: 20_000, at: 11)
    renderer.draw(in: view) // Hidden views stop even with an unfinished envelope.
    #expect(view.isPaused)
    renderer.receiveSpectrum([], maximumFrequency: 20_000, at: 12)
    #expect(renderer.points.isEmpty)
}

private struct FieldImage {
    let width: Int
    let bytes: [UInt8]

    subscript(x: Int, y: Int) -> SIMD4<UInt8> {
        let offset = (y * width + x) * 4
        return SIMD4(bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3])
    }
}

@Test
func rippleFrontsTravelDecayAndRejectHeldBass() {
    var ripples = SpectrumRipples()
    ripples.advance(to: 0, speed: 200, decay: 500)
    ripples.observe(1, at: 0)
    var stepped = ripples
    ripples.advance(to: 0.5, speed: 200, decay: 500)
    for step in 1...10 { stepped.advance(to: Double(step) / 20, speed: 200, decay: 500) }
    #expect(ripples.waves[0].x == 100)
    #expect(abs(ripples.waves[0].y - exp(-1.0)) < 0.000001)
    #expect(abs(stepped.waves[0].y - ripples.waves[0].y) < 0.000001)
    ripples.observe(1, at: 0.5)
    #expect(ripples.waves.count == 1)
    ripples.observe(0, at: 0.6)
    ripples.observe(0.5, at: 0.7)
    #expect(ripples.waves.count == 2)
    #expect(ripples.waves[1].y == 0.5)
    ripples.retire(beyond: 99)
    #expect(ripples.waves.count == 1)
    ripples.advance(to: 10, speed: 200, decay: 500)
    #expect(ripples.waves.isEmpty)
    for _ in 0..<6 { ripples.observe(0, at: 10, preview: true) }
    #expect(ripples.waves.count == 4)
}

@Test @MainActor
func ripplesWarpDotsAndSilhouetteNearTravelingFrontAndStop() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 0), SIMD2(1, 0)]
    renderer.style = SpectrumStyle(shading: .halftone, dotSize: 2, dotSpacing: 10,
                                   amplitudeResponse: 0, chromaticEnabled: false, whiteHalftone: true,
                                   rippleEnabled: true, rippleStrength: 12,
                                   rippleSpeed: 100, rippleWidth: 20, rippleDecay: 2000)
    let baseline = try await fieldPixels(renderer)
    renderer.receiveBass([1, 0], at: 0)
    renderer.advanceRipples(to: 0.255) // At x=15.5, phase −0.5 shifts upward by ~5.28pt.
    let early = try await fieldPixels(renderer)
    #expect(baseline[15, 49].w == 0)
    #expect(early[15, 49].w > 200)
    #expect(early[15, 55].w == 0) // Dot moved, not enlarged.
    #expect(early[35, 50].w > 200) // Opposite lobe moves downward.
    #expect(early[15, 29].w > 200) // Silhouette moves above the original y=30 boundary.
    #expect(early[75, 55].w == baseline[75, 55].w)
    renderer.advanceRipples(to: 0.755)
    let late = try await fieldPixels(renderer)
    #expect(late[15, 55].w == baseline[15, 55].w)
    #expect(late[65, 50].w > 150)
    renderer.style.rippleStrength = 0
    let zero = try await fieldPixels(renderer)
    #expect(zero.bytes == baseline.bytes)
    renderer.style.rippleStrength = 12
    let view = MTKView(frame: .zero, device: renderer.device)
    renderer.updateAnimation(view)
    #expect(!view.isPaused)
    renderer.draw(in: view)
    #expect(view.isPaused)
    #expect(renderer.ripples.waves.isEmpty)
    renderer.style.ripplePreview += 1
    renderer.receiveBass([], at: 1)
    #expect(renderer.ripples.waves.count == 1)
    renderer.style.rippleEnabled = false
    renderer.receiveBass([], at: 1.1)
    renderer.updateAnimation(view)
    #expect(view.isPaused)
    #expect(renderer.ripples.waves.isEmpty)
}

/// Render the real shader at a fixed logical size, with independently chosen geometry.
@MainActor
private func fieldPixels(_ renderer: SpectrumRenderer, scale: Int = 1) async throws -> FieldImage {
    let width = 120 * scale
    let height = 60 * scale
    let descriptor = MTLTextureDescriptor.texture2DDescriptor(
        pixelFormat: .bgra8Unorm, width: width, height: height, mipmapped: false)
    descriptor.storageMode = .shared
    descriptor.usage = .renderTarget
    let texture = try #require(renderer.device.makeTexture(descriptor: descriptor))
    let pass = MTLRenderPassDescriptor()
    pass.colorAttachments[0].texture = texture
    pass.colorAttachments[0].loadAction = .clear
    pass.colorAttachments[0].storeAction = .store
    pass.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 0)
    let command = try #require(renderer.queue.makeCommandBuffer())
    renderer.encode(pass: pass, command: command, scale: Float(scale))
    await withCheckedContinuation { continuation in
        command.addCompletedHandler { _ in continuation.resume() }
        command.commit()
    }
    #expect(command.status == .completed)
    var pixels = [UInt8](repeating: 0, count: width * height * 4)
    pixels.withUnsafeMutableBytes {
        texture.getBytes($0.baseAddress!, bytesPerRow: width * 4,
                         from: MTLRegionMake2D(0, 0, width, height), mipmapLevel: 0)
    }
    return FieldImage(width: width, bytes: pixels)
}

@Test @MainActor
func distanceMappingsUseSignedEuclideanDistanceAndLiveParameters() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 0), SIMD2(1, 0)] // Horizontal at logical y = 30.
    renderer.style = SpectrumStyle(shading: .line, hue: 0)
    let line = try await fieldPixels(renderer)
    #expect(line[60, 29].z > 200) // Red line (BGRA).
    #expect(line[60, 26].w == 0)
    renderer.style.width = 8
    renderer.style.hue = 1 / 3
    let wide = try await fieldPixels(renderer)
    #expect(wide[60, 26].w > 200)
    #expect(wide[60, 29].y > 200) // Hue changed to green.
    #expect(wide[60, 29].z == 0)
    renderer.style = SpectrumStyle(shading: .glow, spread: 2)
    let tight = try await fieldPixels(renderer)
    renderer.style.spread = 10
    let glow = try await fieldPixels(renderer)
    #expect(Int(glow[60, 20].w) > Int(tight[60, 20].w) + 40)
    for i in stride(from: 0, to: glow.bytes.count, by: 4) {
        let premultiplied = glow.bytes[i...i + 2].allSatisfy { $0 <= glow.bytes[i + 3] }
        #expect(premultiplied)
    }
    renderer.style.shading = .bands
    let bands = try await fieldPixels(renderer)
    #expect(bands[60, 20].w > 100) // Ring 10 pt away.
    #expect(bands[60, 24].w == 0) // Gap between rings.
    renderer.style.shading = .distance
    let signed = try await fieldPixels(renderer)
    #expect(signed[60, 10].x > signed[60, 10].z) // Cyan above.
    #expect(signed[60, 50].x < signed[60, 50].z) // Orange below.

    renderer.style = SpectrumStyle(shading: .line, width: 6)
    renderer.points = [SIMD2(-1, -1), SIMD2(-0.5, 1), SIMD2(1, -1)]
    let slope = try await fieldPixels(renderer)
    // First segment is y = 60 − 2x. This pixel is 5.5 pt above it vertically,
    // but only 5.5/√5 ≈ 2.46 pt perpendicular: inside the 3 pt half-width.
    #expect(slope[10, 33].w > 200)
    #expect(slope[10, 27].w == 0)

    renderer.points = [SIMD2(-1, 0), SIMD2(1, 0)]
    renderer.style.width = 2
    let retina = try await fieldPixels(renderer, scale: 2)
    #expect(retina[120, 59].w == 255)
    #expect(retina[120, 56].w == 0)
    renderer.points = []
    for shading in SpectrumShading.allCases {
        renderer.style.shading = shading
        let empty = try await fieldPixels(renderer)
        #expect(empty.bytes.allSatisfy { $0 == 0 })
    }
}

@Test @MainActor
func halftoneBoundaryShrinksWholeCirclesInsteadOfClipping() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.style = SpectrumStyle(shading: .halftone, dotSize: 8, dotSpacing: 10,
                                   amplitudeResponse: 0, whiteHalftone: true)
    // Boundary y=33, only 2pt above the dot at (55,35). Its old 4pt
    // radius would be sliced; the new radius is 2 minus the AA fringe.
    renderer.points = [SIMD2(-1, -0.1), SIMD2(1, -0.1)]
    let flat = try await fieldPixels(renderer)
    #expect(flat[55, 34].w > 200)
    #expect(flat[55, 34].w == flat[55, 35].w)
    #expect(flat[54, 35].w == flat[55, 35].w)
    #expect(flat[57, 35].w == 0) // Shrunk sides, not just a masked top.
    #expect(flat[55, 32].w == 0)
    #expect(flat[57, 45].w == 255) // Interior dots retain their size.
    // y=60−0.5x: center (55,35) has perpendicular clearance 2.5/√1.25.
    renderer.points = [SIMD2(-1, -1), SIMD2(1, 1)]
    let slope = try await fieldPixels(renderer)
    #expect(slope[57, 35].w == 0)
    #expect(slope[55, 34].w == slope[55, 35].w)
    let retina = try await fieldPixels(renderer, scale: 2)
    #expect(retina[110, 69].w == retina[110, 70].w)
    #expect(retina[115, 70].w == 0)
}

@Test @MainActor
func halftoneMasksDotsAndControlsTheirLatticeAndAmplitude() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 0), SIMD2(1, 0)] // Filled area is the lower half.
    renderer.style = SpectrumStyle(shading: .halftone, dotSize: 4, dotSpacing: 10, amplitudeResponse: 0)
    let grid = try await fieldPixels(renderer)
    #expect(grid[5, 55].w == 255) // Centers are (5 + 10c, 55 − 10r).
    #expect(grid[5, 25].w == 0) // Same lattice, above the spectrum.
    #expect(grid[10, 55].w == 0) // Gap, not a solid fill.
    #expect(grid[8, 55].w == 0)
    renderer.style.dotSize = 8
    let larger = try await fieldPixels(renderer)
    #expect(larger[8, 55].w > 200)
    renderer.style.dotSize = 4
    renderer.style.dotSpacing = 12
    let spaced = try await fieldPixels(renderer)
    #expect(spaced[18, 54].w == 255)
    #expect(spaced[15, 55].w == 0)

    renderer.style.dotSpacing = 10
    renderer.style.dotPattern = .hex
    let hex = try await fieldPixels(renderer)
    // Odd rows shift by half the spacing; row separation is 10√3/2, not 10.
    #expect(hex[10, 46].w == 255)
    #expect(grid[10, 46].w == 0)
    #expect(hex[5, 45].w == 0)
    #expect(grid[5, 45].w == 255)
    #expect(hex[5, 37].w > 200)
    #expect(grid[5, 37].w < 50)
    let retina = try await fieldPixels(renderer, scale: 2)
    #expect(retina[20, 92].w == 255)
    #expect(retina[10, 90].w == 0)

    renderer.points = [SIMD2(-1, -0.5), SIMD2(1, 0.5)]
    renderer.style = SpectrumStyle(shading: .halftone, dotSize: 8, dotSpacing: 10, amplitudeResponse: 1)
    let reactive = try await fieldPixels(renderer)
    // Dot diameter follows height at the dot center, not the fragment's x or y.
    #expect(reactive[17, 55].w == 0)
    // Center x=105 gives radius 2.75; this pixel is √6.5 away, ~69.6% coverage.
    #expect(abs(Int(reactive[107, 55].w) - 177) <= 1)
    #expect(reactive[107, 55].w == reactive[107, 45].w)
    renderer.style.amplitudeResponse = 0
    let uniform = try await fieldPixels(renderer)
    #expect(uniform[17, 55].w == 255)
    #expect(uniform[107, 55].w == 255)
    for i in stride(from: 0, to: reactive.bytes.count, by: 4) {
        let premultiplied = reactive.bytes[i...i + 2].allSatisfy { $0 <= reactive.bytes[i + 3] }
        #expect(premultiplied)
    }
    renderer.points = [SIMD2(-1, -1), SIMD2(1, -1)]
    let silent = try await fieldPixels(renderer)
    #expect(silent.bytes.allSatisfy { $0 == 0 })
}

@Test @MainActor
func halftoneVerticalResponseTapersWithinTheLocalSpectrumArea() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 0), SIMD2(1, 0)] // 30pt filled area in a 60pt view.
    for pattern in HalftonePattern.allCases {
        renderer.style = SpectrumStyle(shading: .halftone, dotSize: 8, dotSpacing: 10,
                                       amplitudeResponse: 0, dotPattern: pattern, verticalResponse: 1)
        let topRow = pattern == .grid ? 35 : 37
        let top = try await fieldPixels(renderer)
        #expect(top[7, topRow].w > 200)
        #expect(top[7, 55].w == 0)
        renderer.style.verticalResponse = -1
        let bottom = try await fieldPixels(renderer)
        #expect(bottom[7, topRow].w == 0)
        #expect(bottom[7, 55].w > 200)
        renderer.style.verticalResponse = 0
        let uniform = try await fieldPixels(renderer)
        #expect(uniform[7, topRow].w == 255)
        #expect(uniform[7, 55].w == 255)
        renderer.style.verticalResponse = 0.5
        let partial = try await fieldPixels(renderer)
        #expect(partial[7, 55].w > 0)
        #expect(partial[7, 55].w < 200)
        renderer.style.verticalResponse = 1
        renderer.style.amplitudeResponse = 1
        let combined = try await fieldPixels(renderer)
        #expect(combined[7, topRow].w == 0) // Amplitude still independently scales diameter.
    }
    renderer.points = [SIMD2(-1, -1), SIMD2(1, -1)]
    for vertical in [-1.0, 1.0] {
        renderer.style.verticalResponse = vertical
        let silent = try await fieldPixels(renderer)
        #expect(silent.bytes.allSatisfy { $0 == 0 })
    }
}

@Test
func bassPulseRejectsTrebleAndReleasesWithoutRetriggeringHeldNotes() {
    var bins = [Float](repeating: 0, count: 256)
    bins[82] = 1 // Just above the 180 Hz cutoff.
    #expect(bassDrive(bins, threshold: -42) == 0)
    bins[81] = 0.001 // Within bass, below threshold.
    #expect(bassDrive(bins, threshold: -42) == 0)
    bins[81] = 0.1
    #expect(bassDrive(bins, threshold: -42) == 1)
    #expect(bassDrive(bins, threshold: -12) == 0)
    #expect(bassDrive([], threshold: -42) == 0)
    var pulse = BassPulse()
    pulse.observe(1, at: 0, decay: 250)
    #expect(pulse.level == 1)
    pulse.observe(1, at: 0.25, decay: 250)
    #expect(abs(pulse.level - exp(-1)) < 0.0001)
    var subdivided = BassPulse()
    subdivided.observe(1, at: 0, decay: 250)
    for frame in 1...25 { subdivided.advance(to: Double(frame) / 100, decay: 250) }
    #expect(abs(pulse.level - subdivided.level) < 0.0001)
    pulse.observe(0, at: 0.3, decay: 250)
    pulse.observe(0.7, at: 0.4, decay: 250)
    #expect(pulse.level == 0.7)
    pulse.advance(to: 3, decay: 250)
    #expect(pulse.level == 0)
}

@Test @MainActor
func bassChromaticSplitAffectsBothEndsAndStopsWhenDisabledOrHidden() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 0), SIMD2(1, 0)]
    renderer.style = SpectrumStyle(shading: .halftone, hue: 5 / 6, dotSize: 2, dotSpacing: 10,
                                   amplitudeResponse: 0, chromaticEnabled: true, chromaticStrength: 3,
                                   whiteHalftone: false, rippleEnabled: false)
    let baseline = try await fieldPixels(renderer)
    var bins = [Float](repeating: 0, count: 256)
    bins[30] = 1
    renderer.receiveBass(bins, at: 0)
    let split = try await fieldPixels(renderer)
    for center in [5, 105] { // Bass triggers displacement even at the treble end.
        #expect(baseline[center + 3, 55].w == 0)
        #expect(split[center + 3, 55].z > 150) // Red shifted right.
        #expect(split[center + 3, 55].x == 0)
        #expect(split[center - 3, 55].x > 150) // Blue shifted left.
        #expect(split[center - 3, 55].z == 0)
    }
    for i in stride(from: 0, to: split.bytes.count, by: 4) {
        let premultiplied = split.bytes[i...i + 2].allSatisfy { $0 <= split.bytes[i + 3] }
        #expect(premultiplied)
    }
    renderer.style.chromaticEnabled = false
    renderer.receiveBass(bins, at: 0.1)
    let disabled = try await fieldPixels(renderer)
    #expect(disabled.bytes == baseline.bytes)
    #expect(renderer.bassPulse.level == 0)
    renderer.style.chromaticEnabled = true
    renderer.receiveBass(bins, at: 0.2)
    let view = MTKView(frame: .zero, device: renderer.device)
    renderer.updateAnimation(view)
    #expect(!view.isPaused)
    renderer.draw(in: view) // Detached views must stop their animation loop.
    #expect(view.isPaused)
    #expect(renderer.bassPulse.level == 0)
}

@Test @MainActor
func whiteHalftonePreservesCoverageAndSplitsAllThreeChannels() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 0), SIMD2(1, 0)]
    renderer.style = SpectrumStyle(shading: .halftone, hue: 0, dotSize: 2, dotSpacing: 10,
                                   amplitudeResponse: 0, whiteHalftone: true, rippleEnabled: false)
    let white = try await fieldPixels(renderer)
    #expect(white[105, 55].w > 150)
    for i in stride(from: 0, to: white.bytes.count, by: 4) {
        let pixel = Array(white.bytes[i...i + 3])
        #expect(pixel.allSatisfy { $0 == pixel[3] }) // White premultiplied by coverage.
    }
    renderer.style.whiteHalftone = false
    let hue = try await fieldPixels(renderer)
    #expect(hue[105, 55].x == 0 && hue[105, 55].y == 0) // Original red hue retained.
    #expect(hue[105, 55].z == white[105, 55].w)
    for i in stride(from: 3, to: white.bytes.count, by: 4) {
        #expect(hue.bytes[i] == white.bytes[i])
    }
    renderer.style.whiteHalftone = true
    renderer.style.chromaticEnabled = true
    renderer.style.chromaticStrength = 3
    renderer.receiveBass([1, 0], at: 0)
    let split = try await fieldPixels(renderer)
    #expect(split[108, 55].z > 150 && split[108, 55].x == 0)
    #expect(split[105, 55].y > 150 && split[105, 55].z == 0)
    #expect(split[102, 55].x > 150 && split[102, 55].y == 0)
}

@MainActor @Observable
private final class LabState {
    var style = SpectrumStyle()
    var demo = true
    var spectrum = demoSpectrum
    var scheme = ColorScheme.dark
}

private struct LabPreview: View {
    @Bindable var state: LabState

    var body: some View {
        VStack(spacing: 12) {
            SpectrumView(spectrum: state.demo ? state.spectrum : [], maximumFrequency: 20_000,
                         style: state.style)
                .frame(height: 160)
            SpectrumDebugPanel(style: $state.style, demo: $state.demo)
                .padding(.horizontal, 12)
        }
        .padding(.bottom, 12)
        .frame(width: 320)
        .background(Color(nsColor: .windowBackgroundColor))
        .environment(\.colorScheme, state.scheme)
    }
}

@MainActor
private func findMetalView(_ view: NSView) -> MTKView? {
    if let metal = view as? MTKView { return metal }
    return view.subviews.lazy.compactMap(findMetalView).first
}

@Test @MainActor
func spectrumSpansContentWidth() async throws {
    _ = NSApplication.shared
    let model = TunicModel(connectAudio: false)
    defer { model.shutdown() }
    for _ in 0..<100 where model.snapshot == nil {
        try await Task.sleep(for: .milliseconds(10))
    }
    let preset = try #require(model.snapshot?.presets.first { $0.model == "HD650" })
    model.enqueue(.usePreset(id: preset.id))
    for _ in 0..<100 where model.snapshot?.presetId != preset.id {
        try await Task.sleep(for: .milliseconds(10))
    }
    #expect(model.snapshot?.controls.isEmpty == false)
    let host = NSHostingView(rootView: ContentView(model: model)
        .background(Color(nsColor: .windowBackgroundColor)).environment(\.colorScheme, .dark))
    let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 320, height: 500),
                          styleMask: [.borderless], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentView = host
    window.setContentSize(host.fittingSize)
    defer { window.close() }
    host.layoutSubtreeIfNeeded()
    let metal = try #require(findMetalView(host))
    let frame = metal.convert(metal.bounds, to: host)
    #expect(frame.minX == 0)
    #expect(frame.width == 320)
    #expect(frame.height == 160)
    if let directory = ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"] {
        window.orderFrontRegardless()
        try await Task.sleep(for: .milliseconds(250))
        // Use fixture bins in the real ContentView's renderer without requiring live audio.
        let renderer = try #require(metal.delegate as? SpectrumRenderer)
        renderer.points = spectrumPoints(demoSpectrum, maximumFrequency: 20_000)
        metal.draw()
        try await Task.sleep(for: .milliseconds(250))
        let capture = Process()
        capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        capture.arguments = ["-x", "-o", "-l", String(window.windowNumber),
                             directory + "/native-full-width.png"]
        try capture.run()
        capture.waitUntilExit()
        #expect(capture.terminationStatus == 0)
    }
}

@Test(.enabled(if: ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"] != nil)) @MainActor
func renderVisualizerLab() async throws {
    let directory = try #require(ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"])
    _ = NSApplication.shared
    let state = LabState()
    let host = NSHostingView(rootView: LabPreview(state: state))
    let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 320, height: 360),
                          styleMask: [.borderless], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentView = host
    window.setContentSize(host.fittingSize)
    window.orderFrontRegardless()
    defer { window.close() }
    // Mutate the existing bound state, without recreating the hosting view or renderer.
    let cases: [(String, SpectrumStyle, Bool, ColorScheme)] = SpectrumShading.allCases.map {
        ($0.label, SpectrumStyle(shading: $0), true, .dark)
    } + [
        ("Halftone-Hex", SpectrumStyle(shading: .halftone, dotPattern: .hex), true, .dark),
        ("Halftone-Top", SpectrumStyle(shading: .halftone, dotSize: 5, amplitudeResponse: 0,
                                      verticalResponse: 1), true, .dark),
        ("Halftone-Bottom", SpectrumStyle(shading: .halftone, dotSize: 5, amplitudeResponse: 0,
                                         dotPattern: .hex, verticalResponse: -1), true, .dark),
        ("Halftone-Uniform", SpectrumStyle(shading: .halftone, amplitudeResponse: 0), true, .dark),
        ("Halftone-Round-Boundary", SpectrumStyle(shading: .halftone, dotSize: 8, dotSpacing: 10,
                                                  amplitudeResponse: 0, whiteHalftone: true), true, .dark),
        ("Halftone-Light", SpectrumStyle(shading: .halftone), true, .light),
        ("Halftone-Chromatic", SpectrumStyle(shading: .halftone, hue: 0.85, dotSize: 4, dotSpacing: 9,
                                            amplitudeResponse: 0, chromaticEnabled: true,
                                            chromaticStrength: 8, chromaticDecay: 1000), true, .dark),
        ("Halftone-White", SpectrumStyle(shading: .halftone, whiteHalftone: true), true, .dark),
        ("Halftone-White-Chromatic", SpectrumStyle(shading: .halftone, dotSize: 4, dotSpacing: 9,
                                                  amplitudeResponse: 0, chromaticEnabled: true,
                                                  chromaticStrength: 3, chromaticDecay: 1000,
                                                  whiteHalftone: true), true, .dark),
        ("Defaults", SpectrumStyle(), true, .dark),
        ("Shape-Before", SpectrumStyle(chromaticEnabled: false, rippleEnabled: false, shapeSmoothing: 0), true, .dark),
        ("Shape-After", SpectrumStyle(chromaticEnabled: false, rippleEnabled: false, shapeSmoothing: 0.5), true, .dark),
        ("Points", SpectrumStyle(mode: .points), true, .dark),
        ("Ripples", SpectrumStyle(shading: .halftone, dotSize: 2, dotSpacing: 7,
                                   amplitudeResponse: 0, whiteHalftone: true,
                                   rippleEnabled: true, rippleStrength: 24, ripplePreview: 1), true, .dark),
        ("Ripples-Hex", SpectrumStyle(shading: .halftone, dotSize: 2, dotSpacing: 7,
                                       amplitudeResponse: 0, dotPattern: .hex, chromaticEnabled: true,
                                       chromaticDecay: 1000, rippleEnabled: true,
                                       rippleStrength: 24, ripplePreview: 2), true, .dark),
        ("Envelope", SpectrumStyle(attack: 150, decay: 800), true, .dark),
        ("Empty", SpectrumStyle(), false, .dark),
        ("Light", SpectrumStyle(), true, .light),
    ]
    for (name, style, demo, scheme) in cases {
        state.style.chromaticEnabled = false
        state.style.rippleEnabled = false
        try await Task.sleep(for: .milliseconds(50))
        state.style = style
        state.demo = demo
        state.spectrum = name.hasPrefix("Shape-") ? (0..<256).map { index in
            if index >= 85 { return demoSpectrum[index] }
            let db = index < 20 ? -45.0 : index < 43 ? -15 : index < 65 ? -30 : -55
            return Float(pow(10, db / 20))
        } : demoSpectrum
        state.scheme = scheme
        if name.hasPrefix("Shape-") { try await Task.sleep(for: .seconds(1)) }
        try await Task.sleep(for: .milliseconds(250))
        window.setContentSize(host.fittingSize)
        try await Task.sleep(for: .milliseconds(100))
        let capture = Process()
        capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        capture.arguments = ["-x", "-o", "-l", String(window.windowNumber),
                             directory + "/lab-\(name).png"]
        try capture.run()
        capture.waitUntilExit()
        #expect(capture.terminationStatus == 0)
        if demo && style.mode == .sdf && style.shading == .halftone && style.chromaticEnabled {
            let metal = try #require(findMetalView(host))
            let renderer = try #require(metal.delegate as? SpectrumRenderer)
            #expect(!metal.isPaused)
            #expect(renderer.bassPulse.level > 0)
            // No new samples: a real MTKView must keep drawing the tail, then stop.
            state.style.chromaticDecay = 50
            state.style.rippleEnabled = false
            try await Task.sleep(for: .milliseconds(650))
            #expect(metal.isPaused)
            #expect(renderer.bassPulse.level == 0)
        }
        if demo && style.mode == .sdf && style.shading == .halftone && style.rippleEnabled && !style.chromaticEnabled {
            let metal = try #require(findMetalView(host))
            let renderer = try #require(metal.delegate as? SpectrumRenderer)
            #expect(!metal.isPaused)
            #expect(!renderer.ripples.waves.isEmpty)
            try await Task.sleep(for: .seconds(2))
            #expect(metal.isPaused)
            #expect(renderer.ripples.waves.isEmpty)
        }
    }
}
