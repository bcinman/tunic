import AppKit
import MetalKit
import Observation
import SwiftUI
import Testing
import TunicEngine
@testable import TunicUI

@Observable private final class MeterPreviewRelease {
    var silent = false
}

@Test(.enabled(if: ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"] != nil)) @MainActor
func renderProfileMeterStates() async throws {
    let directory = try #require(ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"])
    _ = NSApplication.shared
    let model = TunicModel(connectAudio: false)
    defer { model.shutdown() }
    for _ in 0..<100 where model.snapshot == nil {
        try await Task.sleep(for: .milliseconds(10))
    }
    let preset = try #require(model.snapshot?.presets.first { $0.model == "HD650" })
    model.enqueue(.createProfile(device: "system-output", name: "HD650", source: .preset(id: preset.id)))
    for _ in 0..<100 where model.snapshot?.presetId != preset.id {
        try await Task.sleep(for: .milliseconds(10))
    }
    let snapshot = try #require(model.snapshot)
    for (name, left, right) in [("silent", Float(0), Float(0)),
                                ("low", 0.0019952623, 0.0056234133),
                                ("threshold", 0.03162278, 0.17782794),
                                ("stereo", 0.03162278, 0.25118864), ("full", 1, 1), ] {
        let release = MeterPreviewRelease()
        let host = NSHostingView(rootView: VStack(spacing: 12) {
            ProfileHeaderView(state: snapshot, send: model.enqueue) { channel in
                CircularLevelMeter(rms: release.silent ? 0 : channel == .left ? left : right, channel: channel)
            }
            SpectrumView(spectrum: demoSpectrum, maximumFrequency: 20_000, style: SpectrumStyle())
                .frame(height: 160)
            ProfileEditorView(state: snapshot, send: model.enqueue)
        }
        .padding(.bottom, 12).frame(width: 320)
        .background(Color(nsColor: .windowBackgroundColor)).environment(\.colorScheme, .dark))
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 320, height: 360),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.setContentSize(host.fittingSize)
        window.orderFrontRegardless()
        defer { window.close() }
        let phases = name == "full" ? [("", 350), ("-decay", 200), ("-settled", 2_300)] : [("", 350)]
        for (phase, delay) in phases {
            if phase == "-decay" { release.silent = true }
            try await Task.sleep(for: .milliseconds(delay))
            let capture = Process()
            capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
            capture.arguments = ["-x", "-o", "-l", String(window.windowNumber),
                                 directory + "/meters-\(name)\(phase).png", ]
            try capture.run()
            capture.waitUntilExit()
            #expect(capture.terminationStatus == 0)
        }
    }
}

@Test @MainActor
func spectrumAttackAndDecayUseElapsedTimeAndSettle() throws {
    let device = try #require(MTLCreateSystemDefaultDevice())
    let renderer = try SpectrumRenderer(device: device)
    renderer.receiveSpectrum([1, 0.001], maximumFrequency: 20_000, at: 0)
    renderer.advanceSpectrum(to: 0.01) // Fixed 10 ms attack: one time constant.
    #expect(abs(renderer.points[0].y - Float(1 - 2 / exp(Double(1)))) < 0.00001)
    #expect(abs(renderer.points[1].y - Float(-1 + (2.0 / 3) * (1 - 1 / exp(Double(1))))) < 0.00001)
    let split = try SpectrumRenderer(device: device)
    split.style = renderer.style
    split.receiveSpectrum([1, 0.001], maximumFrequency: 20_000, at: 0)
    for step in 1...10 { split.advanceSpectrum(to: Double(step) / 1000) }
    #expect(abs(split.points[0].y - renderer.points[0].y) < 0.00001)
    let peak = renderer.points[0].y
    renderer.receiveSpectrum([0, 0], maximumFrequency: 20_000, at: 0.01)
    renderer.advanceSpectrum(to: 0.11) // Fixed 100 ms decay: one time constant.
    #expect(abs(renderer.points[0].y - (-1 + (peak + 1) / Float(exp(Double(1))))) < 0.00001)
    let view = MTKView(frame: .zero, device: device)
    renderer.updateAnimation(view)
    #expect(!view.isPaused)
    renderer.advanceSpectrum(to: 10)
    renderer.updateAnimation(view)
    #expect(view.isPaused)
    renderer.receiveSpectrum([1, 1], maximumFrequency: 20_000, at: 10)
    #expect(renderer.points.map(\.y) == [-1, -1])
    renderer.advanceSpectrum(to: 10.01)
    #expect(renderer.points.allSatisfy { $0.y > -1 && $0.y < 1 })
    renderer.draw(in: view) // Hidden views stop even with an unfinished envelope.
    #expect(view.isPaused)
    #expect(renderer.points.isEmpty)
    renderer.advanceSpectrum(to: 100)
    #expect(renderer.points.isEmpty) // Old targets must not reappear on reopening.
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
func halftoneBoundaryShrinksWholeCirclesInsteadOfClipping() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.style = SpectrumStyle(dotSize: 8, dotSpacing: 10,
                                   amplitudeResponse: 0, solidFill: true)
    // Boundary y=33, only 2pt above the dot at (55,35). Its old 4pt
    // radius would be sliced; the new radius is 2 minus the AA fringe.
    renderer.points = [SIMD2(-1, -0.1), SIMD2(1, -0.1)]
    let flat = try await fieldPixels(renderer)
    #expect(flat[55, 34].w > 200)
    #expect(flat[55, 34].w == flat[55, 35].w)
    #expect(flat[54, 35].w == flat[55, 35].w)
    #expect(flat[57, 35].w == 0) // Shrunk sides, not just a masked top.
    #expect(flat[55, 32].w == 0)
    #expect(flat[55, 45].w == 255) // Interior dot centers remain filled despite the taper.
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
func gridFillMapsSizeAndOpacityIndependentlyWithSolidColor() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 0), SIMD2(1, 0)]
    for shape in CellShape.allCases {
        renderer.style = SpectrumStyle(dotSize: 8, dotSpacing: 10, amplitudeResponse: 0,
                                       cellShape: shape, sizeResponse: 0,
                                       solidFill: true, solidColor: Color(red: 1, green: 0.25, blue: 0))
        let constant = try await fieldPixels(renderer)
        #expect(constant[57, 45].w == 255 && constant[57, 55].w == 255)
        renderer.style.opacityResponse = 1
        let opacity = try await fieldPixels(renderer)
        // Cell centers are 15 and 5 points above the bottom of a 30-point fill.
        #expect(abs(Int(opacity[57, 45].w) - 128) <= 1)
        #expect(abs(Int(opacity[57, 55].w) - 43) <= 1)
        #expect(opacity[57, 45].z == opacity[57, 45].w)
        #expect(abs(Int(opacity[57, 45].y) - 32) <= 1 && opacity[57, 45].x == 0)
        renderer.style.sizeResponse = 1
        let both = try await fieldPixels(renderer)
        renderer.style.opacityResponse = 0
        let size = try await fieldPixels(renderer)
        #expect(size[57, 45].w < constant[57, 45].w)
        #expect(abs(Int(both[57, 45].w) * 2 - Int(size[57, 45].w)) <= 1)
        renderer.style.sizeResponse = 0
        renderer.style.opacityResponse = 0.5
        renderer.style.amplitudeResponse = 1
        let level = try await fieldPixels(renderer)
        // Half-height spectrum × halfway up = 0.25; half response gives alpha 0.625.
        #expect(abs(Int(level[57, 45].w) - 159) <= 1)
    }
}

@Test @MainActor
func squareCellsFitSlopesWithoutClippingOrExcessiveShrinking() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.style = SpectrumStyle(dotSize: 8, dotSpacing: 10, cellShape: .square,
                                   sizeResponse: 0, solidFill: true)
    renderer.points = [SIMD2(-1, -1), SIMD2(1, 1)] // y = 60 − x/2.
    // Center (55,35) is 2.5pt below the line. Its top-left corner fits when
    // half-width = 2.5 / 1.5, before subtracting the antialias fringe.
    let normal = try await fieldPixels(renderer)
    #expect(abs(Int(normal[56, 34].w) - 9) <= 1)
    #expect(normal[56, 34].w == normal[56, 35].w)
    let retina = try await fieldPixels(renderer, scale: 2)
    #expect(abs(Int(retina[112, 69].w) - 149) <= 1)
    #expect(retina[113, 69].w == 0)
}

@Test @MainActor
func halftoneMasksDotsAndControlsTheirLatticeAndAmplitude() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 0), SIMD2(1, 0)] // Filled area is the lower half.
    renderer.style = SpectrumStyle(dotSize: 4, dotSpacing: 10, amplitudeResponse: 0)
    let grid = try await fieldPixels(renderer)
    #expect(grid[5, 35].w == 255) // Centers are (5 + 10c, 55 − 10r).
    #expect(grid[5, 25].w == 0) // Same lattice, above the spectrum.
    #expect(grid[10, 55].w == 0) // Gap, not a solid fill.
    #expect(grid[7, 35].w == 0)
    renderer.style.dotSize = 8
    let larger = try await fieldPixels(renderer)
    #expect(larger[7, 35].w > 200)
    renderer.style.dotSize = 4
    renderer.style.dotSpacing = 12
    let spaced = try await fieldPixels(renderer)
    #expect(spaced[18, 42].w > 200)
    #expect(spaced[15, 55].w == 0)

    renderer.points = [SIMD2(-1, -0.5), SIMD2(1, 0.5)]
    renderer.style = SpectrumStyle(dotSize: 8, dotSpacing: 10, amplitudeResponse: 1)
    let reactive = try await fieldPixels(renderer)
    // Dot diameter follows height at the dot center, not the fragment's x or y.
    #expect(reactive[17, 55].w == 0)
    // At y=35 the center is 25pt above the bottom: amplitude × taper gives
    // radius 4 × 25/60. Pixel (106,35) is √2.5 away, ~58.5% AA coverage.
    #expect(abs(Int(reactive[106, 35].w) - 149) <= 1)
    #expect(reactive[106, 55].w == 0)
    renderer.style.amplitudeResponse = 0
    let uniform = try await fieldPixels(renderer)
    #expect(uniform[16, 45].w == 255)
    #expect(reactive[16, 45].w < uniform[16, 45].w)
    #expect(uniform[106, 35].w == 255)
    for i in stride(from: 0, to: reactive.bytes.count, by: 4) {
        let premultiplied = reactive.bytes[i...i + 2].allSatisfy { $0 <= reactive.bytes[i + 3] }
        #expect(premultiplied)
    }
    renderer.points = [SIMD2(-1, -1), SIMD2(1, -1)]
    let silent = try await fieldPixels(renderer)
    #expect(silent.bytes.allSatisfy { $0 == 0 })
}

@Test @MainActor
func overlappingGridFillMatchesCellUnion() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 0.6), SIMD2(1, 0.6)] // Flat top at y=12; 48pt filled.
    for shape in CellShape.allCases {
        for spacing in [3.0, 8.0] {
            renderer.style = SpectrumStyle(dotSize: 12, dotSpacing: spacing,
                                           amplitudeResponse: 0, cellShape: shape,
                                           solidFill: true)
            // Independent reference: enumerate the entire lattice, rather than
            // copying the shader's per-pixel neighbor search.
            var circles: [(Double, Double, Double)] = []
            let rowStep = spacing
            for row in 0...Int(48 / rowStep) {
                let y = spacing / 2 + Double(row) * rowStep
                for column in 0...Int(120 / spacing) {
                    let x = (Double(column) + 0.5) * spacing
                    let radius = min(6 * y / 48, min(x, 120 - x, y, 48 - y) - 0.75)
                    if radius > 0 { circles.append((x, y, radius)) }
                }
            }
            let dots = try await fieldPixels(renderer)
            for y in stride(from: 0, to: 60, by: 3) {
                for x in stride(from: 0, to: 120, by: 5) {
                    var coverage = 0.0
                    if y >= 12 {
                        for (cx, cy, radius) in circles {
                            let deltaX = abs(Double(x) + 0.5 - cx)
                            let deltaY = abs(59.5 - Double(y) - cy)
                            let distance = shape == .circle ? hypot(deltaX, deltaY) : max(deltaX, deltaY)
                            let t = min(1, max(0, (distance - radius + 0.75) / 1.5))
                            coverage = max(coverage, 1 - t * t * (3 - 2 * t))
                        }
                    }
                    #expect(abs(Int(dots[x, y].w) - Int((255 * coverage).rounded())) <= 1)
                }
            }
        }
    }
    renderer.points = [SIMD2(-1, -1), SIMD2(1, -1)]
    let silent = try await fieldPixels(renderer)
    #expect(silent.bytes.allSatisfy { $0 == 0 })
}

@Test @MainActor
func gridFillSizeResponseTapersTowardTheBottomOfTheLocalSpectrumArea() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 0), SIMD2(1, 0)] // 30pt filled area in a 60pt view.
    for shape in CellShape.allCases {
        renderer.style = SpectrumStyle(dotSize: 8, dotSpacing: 10,
                                       amplitudeResponse: 0, cellShape: shape)
        let topRow = 35
        let top = try await fieldPixels(renderer)
        #expect(top[7, topRow].w > 200)
        #expect(top[7, 55].w == 0)
        renderer.style.amplitudeResponse = 1
        let combined = try await fieldPixels(renderer)
        #expect(combined[7, topRow].w == 0) // Amplitude still independently scales diameter.
    }
    renderer.points = [SIMD2(-1, -1), SIMD2(1, -1)]
    let silent = try await fieldPixels(renderer)
    #expect(silent.bytes.allSatisfy { $0 == 0 })
}

@Test @MainActor
func solidAndGradientColorsPreserveHalftoneCoverage() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 0), SIMD2(1, 0)]
    renderer.style = SpectrumStyle(dotSize: 2, dotSpacing: 10,
                                   amplitudeResponse: 0, solidFill: true)
    let white = try await fieldPixels(renderer)
    #expect(white[105, 35].w > 150)
    for i in stride(from: 0, to: white.bytes.count, by: 4) {
        let pixel = Array(white.bytes[i...i + 3])
        #expect(pixel.allSatisfy { $0 == pixel[3] }) // White premultiplied by coverage.
    }
    renderer.style.solidFill = false
    let gradient = try await fieldPixels(renderer)
    #expect(gradient[105, 35].x > gradient[105, 35].y)
    #expect(gradient[105, 35].z > gradient[105, 35].y)
    for i in stride(from: 3, to: white.bytes.count, by: 4) {
        #expect(gradient.bytes[i] == white.bytes[i])
    }
    renderer.style.solidFill = true
    renderer.style.solidColor = .black
    let black = try await fieldPixels(renderer)
    for i in stride(from: 0, to: black.bytes.count, by: 4) {
        #expect(black.bytes[i...i + 2].allSatisfy { $0 == 0 })
        #expect(black.bytes[i + 3] == white.bytes[i + 3])
    }
}

@Test @MainActor
func halftoneGradientInterpolatesAcrossXAtBothScales() async throws {
    let renderer = try SpectrumRenderer(device: #require(MTLCreateSystemDefaultDevice()))
    renderer.points = [SIMD2(-1, 1), SIMD2(1, 1)]
    renderer.style = SpectrumStyle(dotSize: 3, dotSpacing: 4, sizeResponse: 0,
                                   solidFill: false,
                                   gradientStart: Color(red: 1, green: 0, blue: 0),
                                   gradientSecond: Color(red: 0, green: 1, blue: 0),
                                   gradientThird: Color(red: 0, green: 0, blue: 1),
                                   gradientEnd: Color(red: 1, green: 1, blue: 1))
    for scale in [1, 2] {
        let image = try await fieldPixels(renderer, scale: scale)
        // Sample cell centers on both sides of each stop, away from viewport edges.
        for x in [2, 22, 38, 42, 46, 62, 78, 82, 86, 102, 118] {
            let position = (Double(x * scale) + 0.5) / Double(scale)
            let expected: SIMD3<Double>
            if position < 40 {
                expected = SIMD3(0, position / 40, 1 - position / 40) // BGRA
            } else if position < 80 {
                expected = SIMD3((position - 40) / 40, (80 - position) / 40, 0)
            } else {
                expected = SIMD3(1, (position - 80) / 40, (position - 80) / 40)
            }
            for y in [22, 46] {
                let pixel = image[x * scale, y * scale]
                #expect(pixel.w > 0)
                for channel in 0..<3 {
                    #expect(abs(Double(pixel[channel]) - expected[channel] * Double(pixel.w)) < 2)
                }
            }
        }
    }
    renderer.style.gradientEnd = Color(red: 0, green: 0, blue: 0)
    let edited = try await fieldPixels(renderer)
    #expect(edited[118, 46].x < 30 && edited[118, 46].y == 0 && edited[118, 46].z == 0)
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
            VisualizationSettingsView(style: $state.style, demo: $state.demo)
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

@Test(.enabled(if: ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"] != nil)) @MainActor
func renderSpectrumSettlesWhenIdle() async throws {
    let directory = try #require(ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"])
    _ = NSApplication.shared
    let state = LabState()
    state.style.solidFill = false
    state.demo = false
    let host = NSHostingView(rootView: LabPreview(state: state))
    let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 320, height: 600),
                          styleMask: [.borderless], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentView = host
    window.setContentSize(host.fittingSize)
    window.orderFrontRegardless()
    defer { window.close() }
    try await Task.sleep(for: .milliseconds(100))
    let metal = try #require(findMetalView(host))
    let renderer = try #require(metal.delegate as? SpectrumRenderer)
    renderer.receiveSpectrum(demoSpectrum, maximumFrequency: 20_000, at: CACurrentMediaTime())
    renderer.updateAnimation(metal)
    #expect(!metal.isPaused)
    try await Task.sleep(for: .milliseconds(350))
    #expect(metal.isPaused)
    #expect(renderer.points.contains { $0.y > -1 })
    let capture = Process()
    capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    capture.arguments = ["-x", "-o", "-l", String(window.windowNumber),
                         directory + "/spectrum-idle.png", ]
    try capture.run()
    capture.waitUntilExit()
    #expect(capture.terminationStatus == 0)
    window.orderOut(nil)
    #expect(metal.isPaused)
}

@Test @MainActor
func nativeVisualizerHostsMetalView() async throws {
    _ = NSApplication.shared
    let model = TunicModel(connectAudio: false)
    defer { model.shutdown() }
    for _ in 0..<100 where model.snapshot == nil {
        try await Task.sleep(for: .milliseconds(10))
    }
    let preset = try #require(model.snapshot?.presets.first { $0.model == "HD650" })
    model.enqueue(.createProfile(device: "system-output", name: "HD650", source: .preset(id: preset.id)))
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
                             directory + "/native-full-width.png", ]
        try capture.run()
        capture.waitUntilExit()
        #expect(capture.terminationStatus == 0)
    }
    // Exercise the overlay toggle in both directions, without replacing the renderer.
    window.orderFrontRegardless()
    let collapsedSize = host.fittingSize
    try await Task.sleep(for: .milliseconds(100))
    for expanded in [true, false] {
        // The chevron overlays the bottom-right corner of the spectrum.
        let location = metal.convert(NSPoint(x: metal.bounds.width - 22,
                                             y: metal.isFlipped ? metal.bounds.height - 22 : 22), to: nil)
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = try #require(NSEvent.mouseEvent(with: type, location: location,
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, eventNumber: 0,
                clickCount: 1, pressure: 1))
            window.sendEvent(event)
        }
        try await Task.sleep(for: .milliseconds(500))
        let size = host.fittingSize
        let expandedHeight = collapsedSize.height + 260
        #expect(size.width == collapsedSize.width)
        #expect(size.height == (expanded ? expandedHeight : collapsedSize.height))
        #expect(findMetalView(host) === metal)
        #expect(metal.bounds.height == 140)
        let top = window.frame.maxY
        window.setContentSize(size)
        window.setFrameTopLeftPoint(NSPoint(x: window.frame.minX, y: top))
        try await Task.sleep(for: .milliseconds(250))
        if let directory = ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"] {
            let renderer = try #require(metal.delegate as? SpectrumRenderer)
            renderer.receiveSpectrum(demoSpectrum, maximumFrequency: 20_000, at: CACurrentMediaTime())
            metal.draw()
            try await Task.sleep(for: .milliseconds(250))
            let capture = Process()
            capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
            capture.arguments = ["-x", "-o", "-l", String(window.windowNumber),
                                 directory + "/visualizer-\(expanded ? "expanded" : "collapsed").png", ]
            try capture.run()
            capture.waitUntilExit()
            #expect(capture.terminationStatus == 0)
        }
    }
}

@Test(.enabled(if: ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"] != nil)) @MainActor
func renderVisualizerLab() async throws {
    let directory = try #require(ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"])
    _ = NSApplication.shared
    try await renderProfileCreationLab(directory: directory)
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
    let cases: [(String, SpectrumStyle, Bool, ColorScheme)] = [
        ("Grid-Square", SpectrumStyle(dotSize: 6, dotSpacing: 8, cellShape: .square), true, .dark),
        ("Grid-Opacity", SpectrumStyle(dotSize: 4, dotSpacing: 6, sizeResponse: 0, opacityResponse: 1), true, .dark),
        ("Grid-Both", SpectrumStyle(dotSize: 6, dotSpacing: 8, opacityResponse: 1), true, .dark),
        ("Grid-Constant-Solid", SpectrumStyle(dotSize: 4, dotSpacing: 6, cellShape: .square,
                                              sizeResponse: 0,
                                              solidFill: true, solidColor: .orange), true, .dark),
        ("Halftone-Overlap", SpectrumStyle(dotSize: 12, dotSpacing: 6), true, .dark),
        ("Halftone-Taper-Grid", SpectrumStyle(dotSize: 5,
                                             amplitudeResponse: 0), true, .dark),
        ("Halftone-Round-Boundary", SpectrumStyle(dotSize: 8, dotSpacing: 10,
                                                  amplitudeResponse: 0, solidFill: true), true, .dark),
        ("Halftone-White", SpectrumStyle(solidFill: true), true, .dark),
        ("Defaults", SpectrumStyle(), true, .dark),
        ("Empty", SpectrumStyle(), false, .dark),
        ("Light", SpectrumStyle(), true, .light),
    ]
    for (name, style, demo, scheme) in cases {
        state.style = style
        state.demo = demo
        state.scheme = scheme
        try await Task.sleep(for: .milliseconds(250))
        window.setContentSize(host.fittingSize)
        try await Task.sleep(for: .milliseconds(100))
        let capture = Process()
        capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        capture.arguments = ["-x", "-o", "-l", String(window.windowNumber),
                             directory + "/lab-\(name).png", ]
        try capture.run()
        capture.waitUntilExit()
        #expect(capture.terminationStatus == 0)
    }

    // Reuse the same window and renderer, as MenuBarExtra does. No new input is
    // delivered between closing and reopening: the retained drawable must stay hidden.
    let metal = try #require(findMetalView(host))
    let renderer = try #require(metal.delegate as? SpectrumRenderer)
    #expect(!renderer.points.isEmpty)
    #expect(metal.layer?.isHidden == false)
    window.orderOut(nil)
    try await Task.sleep(for: .milliseconds(100))
    #expect(renderer.points.isEmpty)
    #expect(metal.isPaused)
    #expect(metal.layer?.isHidden == true)
    window.orderFrontRegardless()
    try await Task.sleep(for: .milliseconds(100))
    #expect(metal.layer?.isHidden == true)
    renderer.receiveSpectrum(Array(repeating: 0, count: 256), maximumFrequency: 20_000,
                             at: CACurrentMediaTime())
    metal.draw()
    try await Task.sleep(for: .milliseconds(100))
    #expect(renderer.points.allSatisfy { $0.y == -1 })
    #expect(metal.layer?.isHidden == false)
    let capture = Process()
    capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    capture.arguments = ["-x", "-o", "-l", String(window.windowNumber),
                         directory + "/lab-Reopened-Silent.png", ]
    try capture.run()
    capture.waitUntilExit()
    #expect(capture.terminationStatus == 0)
    renderer.receiveSpectrum(demoSpectrum, maximumFrequency: 20_000, at: CACurrentMediaTime())
    metal.draw()
    #expect(renderer.points.contains { $0.y > -1 })
    try await renderGainHoverStates(directory: directory)
    try await nativeVisualizerHostsMetalView()
}

@MainActor
private func renderGainHoverStates(directory: String) async throws {
    let model = TunicModel(connectAudio: false)
    defer { model.shutdown() }
    for _ in 0..<100 where model.snapshot == nil {
        try await Task.sleep(for: .milliseconds(10))
    }
    let preset = try #require(model.snapshot?.presets.first { $0.model == "HD650" })
    model.enqueue(.createProfile(device: "system-output", name: "HD650", source: .preset(id: preset.id)))
    for _ in 0..<100 where model.snapshot?.presetId != preset.id {
        try await Task.sleep(for: .milliseconds(10))
    }
    let snapshot = try #require(model.snapshot)
    let shelf = try #require(snapshot.controls.first)
    let peak = try #require(snapshot.controls.dropFirst().first)
    for (name, filter) in [("Resting", nil), ("Short-Names", nil), ("Long-Names", nil), ("Processing-Off", nil),
                           ("Shelf", shelf.filter), ("Peak", peak.filter), ] {
        var display = snapshot
        display.deviceName = name == "Long-Names"
            ? "External Headphones through USB Audio Interface" : "External Headphones"
        display.profileName = name == "Long-Names" ? "Sennheiser HD6XX with custom bass adjustment" : "Sennheiser HD6XX"
        if name == "Short-Names" {
            display.deviceName = "USB"
            display.profileName = "Flat"
        }
        let host = NSHostingView(rootView: VStack(spacing: 0) {
            ProfileHeaderView(state: display, send: model.enqueue) { channel in
                CircularLevelMeter(rms: 0, channel: channel)
            }
            VisualizerView(model: model, maximumFrequency: 20_000, hoveredFilter: filter)
            ProfileEditorView(state: display, send: model.enqueue)
            HStack {
                AudioStatusView(deviceName: nil, isProcessing: .constant(name != "Processing-Off"))
                Spacer()
                SettingsMenu(quit: {})
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.top, 12)
        }.padding(.bottom, 12).frame(width: 320)
            .background(Color(nsColor: .windowBackgroundColor)).environment(\.colorScheme, .dark))
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 320, height: 500),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.setContentSize(host.fittingSize)
        window.orderFrontRegardless()
        defer { window.close() }
        try await Task.sleep(for: .milliseconds(250))
        let metal = try #require(findMetalView(host))
        let renderer = try #require(metal.delegate as? SpectrumRenderer)
        renderer.receiveSpectrum(demoSpectrum, maximumFrequency: 20_000, at: CACurrentMediaTime())
        metal.draw()
        try await Task.sleep(for: .milliseconds(400))
        let capture = Process()
        capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        capture.arguments = ["-x", "-o", "-l", String(window.windowNumber),
                             directory + "/gain-hover-\(name).png", ]
        try capture.run()
        capture.waitUntilExit()
        #expect(capture.terminationStatus == 0)
    }
}

@Test(arguments: [-12.0, 0, 6])
func gainHighlightFollowsFilterSensitivity(gain: Double) throws {
    let frequencies = [20.0, 200, 1_000, 10_000]
    let peak = try gainInfluence(filter: Filter(id: 2, kind: .peaking, frequency: 1_000,
                                              gain: gain, qualityFactor: 2),
                                 sampleRate: 48_000, frequencies: frequencies)
    #expect(peak[2] > 0.98)
    #expect(peak[0] < 0.01 && peak[3] < 0.02)
    for kind in [FilterKind.lowShelf, .highShelf] {
        let shelf = try gainInfluence(filter: Filter(id: 1, kind: kind, frequency: 200,
                                                   gain: gain, qualityFactor: 0.707),
                                      sampleRate: 48_000, frequencies: frequencies)
        #expect(abs(shelf[1] - 0.5) < 0.02)
        #expect(shelf[kind == .lowShelf ? 0 : 3] > 0.98)
        #expect(shelf[kind == .lowShelf ? 3 : 0] < 0.02)
    }
}
