import AppKit
import MetalKit
import SwiftUI
import Testing
import TunicEngine
@testable import TunicUI

@MainActor
private func waitUntil(_ condition: String = "expected state", _ predicate: () -> Bool) async throws {
    let deadline = ContinuousClock.now + .seconds(5)
    while !predicate() {
        guard ContinuousClock.now < deadline else { throw Timeout(condition: condition) }
        try await Task.sleep(for: .milliseconds(5))
    }
}

private struct Timeout: Error { let condition: String }

@Test
func spectrumUsesDecibelsAndLogarithmicFrequencyCoordinates() {
    let points = spectrumPoints([0, 0.001, 0.1, 2], maximumFrequency: 20_000)
    #expect(points.count == 4)
    for (actual, expected) in zip(points, [
        SIMD2<Float>(-1, -1), SIMD2<Float>(-1 / 3, -1 / 3),
        SIMD2<Float>(1 / 3, 5 / 9), SIMD2<Float>(1, 1),
    ]) {
        #expect(abs(actual.x - expected.x) < 0.001)
        #expect(abs(actual.y - expected.y) < 0.001)
    }
    // A narrower frequency range expands the bins before Metal clips them.
    let cropped = spectrumPoints([0, 1, 0, 0], maximumFrequency: 200)
    #expect(abs(cropped[3].x - 5) < 0.001)
    #expect(spectrumPoints([], maximumFrequency: 20_000).isEmpty)
    #expect(spectrumPoints([1], maximumFrequency: 20_000).isEmpty)
}

@Test @MainActor
func renderSpectrumStates() async throws {
    let device = try #require(MTLCreateSystemDefaultDevice())
    let renderer = try SpectrumRenderer(device: device)
    renderer.style.mode = .points
    let width = 592
    let height = 150
    let descriptor = MTLTextureDescriptor.texture2DDescriptor(
        pixelFormat: .bgra8Unorm, width: width, height: height, mipmapped: false)
    descriptor.storageMode = .shared
    descriptor.usage = .renderTarget
    let texture = try #require(device.makeTexture(descriptor: descriptor))
    let spectrum: [Float] = (0..<256).map { index in
        let x = Double(index) / 255
        let db = -80 + 60 * exp(-pow((x - 0.32) / 0.12, 2))
            + 35 * exp(-pow((x - 0.73) / 0.05, 2))
        return Float(pow(10, db / 20))
    }
    for (name, amplitudes) in [("active", spectrum), ("empty", []), ("silent", [Float](repeating: 0, count: 256))] {
        renderer.points = spectrumPoints(amplitudes, maximumFrequency: 20_000)
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = texture
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].storeAction = .store
        pass.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 0)
        let command = try #require(renderer.queue.makeCommandBuffer())
        renderer.encode(pass: pass, command: command, scale: 2)
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
        let opaque = stride(from: 3, to: pixels.count, by: 4).filter { pixels[$0] != 0 }
        #expect(opaque.count < width * height / 5)
        if name == "empty" {
            #expect(opaque.isEmpty)
        } else {
            #expect(!opaque.isEmpty)
            #expect(opaque.allSatisfy { pixels[$0 - 3...$0].allSatisfy { $0 == 255 } })
        }
        if name == "active" {
            // Bin 82 is near the taller peak (~−20 dBFS), not vertically inverted.
            let offset = (33 * width + 190) * 4
            #expect(pixels[offset + 3] == 255)
            #expect(pixels[(116 * width + 190) * 4 + 3] == 0)
        }
    }
    if let directory = ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"] {
        _ = NSApplication.shared
        let host = NSHostingView(rootView: VStack(spacing: 16) {
            SpectrumView(spectrum: spectrum, maximumFrequency: 20_000, style: SpectrumStyle(mode: .points))
            SpectrumView(spectrum: [], maximumFrequency: 20_000, style: SpectrumStyle(mode: .points))
            SpectrumView(spectrum: [Float](repeating: 0, count: 256), maximumFrequency: 20_000,
                         style: SpectrumStyle(mode: .points))
        }
        .frame(width: 296, height: 257).padding(12)
        .background(Color(red: 0.094, green: 0.125, blue: 0.188)))
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 320, height: 281),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.close() }
        try await Task.sleep(for: .milliseconds(500))
        let capture = Process()
        capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        capture.arguments = ["-x", "-o", "-l", String(window.windowNumber),
                             directory + "/metal-hosted.png", ]
        try capture.run()
        capture.waitUntilExit()
        #expect(capture.terminationStatus == 0)
    }
}

@Test @MainActor
func swiftModelUsesRustDraftsAndReceivesFailedCommands() async throws {
    let model = TunicModel(connectAudio: false)
    defer { model.shutdown() }
    try await waitUntil { model.snapshot != nil }
    let preset = try #require(model.snapshot?.presets.first { $0.model == "HD650" })
    model.enqueue(.usePreset(id: preset.id))
    try await waitUntil { model.snapshot?.presetId == preset.id }
    let control = try #require(model.snapshot?.controls.first)
    let originalResponse = model.response
    model.enqueue(.setControlGain(filter: control.filter, gain: -3.5))
    try await waitUntil { model.snapshot?.controls.first?.gain == -3.5 }
    #expect(model.snapshot?.hasDraft == true)
    #expect(model.response != originalResponse)
    #expect(model.response.count == 128)

    model.enqueue(.usePreset(id: "missing"))
    try await waitUntil { model.snapshot?.actionError != nil }
    #expect(model.snapshot?.controls.first?.gain == -3.5)
    model.enqueue(.resetDraft)
    try await waitUntil { model.snapshot?.hasDraft == false }
    #expect(model.snapshot?.controls.first?.gain == 0)
    #expect(model.snapshot?.actionError == nil)

    model.enqueue(.setControlGain(filter: control.filter, gain: 2.25))
    try await waitUntil { model.snapshot?.hasDraft == true }
    model.enqueue(.saveDraft)
    try await waitUntil { model.snapshot?.hasDraft == false }
    #expect(model.snapshot?.controls.first?.gain == 2.25)
    model.enqueue(.editFilter(filter: 0, frequency: 100, gain: 0))
    #expect(model.error != nil)
}

@Test @MainActor
func updateStreamsHaveIndependentLifetimesAndFinishAtShutdown() async throws {
    let engine = try Engine(connectAudio: false)
    defer { engine.shutdown() }
    var firstCount = 0
    let first = Task {
        for await _ in engine.updates() { firstCount += 1 }
    }
    var secondRevision: UInt64 = 0
    var finished = false
    let second = Task {
        for await _ in engine.updates() {
            secondRevision = engine.snapshot()?.revision ?? 0
        }
        finished = true
    }
    defer { first.cancel(); second.cancel() }
    try await waitUntil { firstCount > 0 && secondRevision > 0 }
    first.cancel()
    await first.value
    let stoppedCount = firstCount
    for _ in 0..<100 { try engine.enqueue(command: .useFlat) }
    try await waitUntil { secondRevision >= 101 }
    #expect(firstCount == stoppedCount)
    #expect(engine.snapshot()?.profileName != nil)
    engine.shutdown()
    try await waitUntil { finished }
    #expect(throws: EngineError.self) { try engine.enqueue(command: .useFlat) }
    // Subscribing after shutdown must complete rather than hang.
    for await _ in engine.updates() { Issue.record("Closed engine emitted an update") }
}

@Test @MainActor
func foreignResponseAndTelemetryTypesRoundTrip() throws {
    let engine = try Engine(connectAudio: false)
    defer { engine.shutdown() }
    let batch = engine.pollTelemetry()
    #expect(batch.frames.isEmpty)
    let chain = Chain(preamp: -9, filters: [Filter(
        id: 9, kind: .peaking, frequency: 1234, gain: 6, qualityFactor: 0.8
    ), ])
    let points = try frequencyResponse(chain: chain, sampleRate: 48000, frequencies: [100, 1234, 10000])
    #expect(abs(points[1] - 6) < 0.001)
    #expect(points[0] < 0.2 && points[2] < 0.2)
    #expect(throws: EngineError.self) {
        try frequencyResponse(chain: chain, sampleRate: 0, frequencies: [100])
    }
}

/// Opt-in renders use real Rust snapshots, never a separate UI fixture model.
@Test(.enabled(if: ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"] != nil)) @MainActor
func renderNativeStates() async throws {
    guard let directory = ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"] else { return }
    let model = TunicModel(connectAudio: false)
    defer { model.shutdown() }
    try await waitUntil { model.snapshot != nil }
    try await render(model, to: directory, name: "empty")
    let preset = try #require(model.snapshot?.presets.first { $0.model == "HD650" })
    model.enqueue(.usePreset(id: preset.id))
    try await waitUntil { model.snapshot?.presetId == preset.id }
    let control = try #require(model.snapshot?.controls.first)
    model.enqueue(.setControlGain(filter: control.filter, gain: 3))
    try await waitUntil { model.snapshot?.hasDraft == true }
    try await render(model, to: directory, name: "edited")
    model.enqueue(.usePreset(id: "missing"))
    try await waitUntil { model.snapshot?.actionError != nil }
    try await render(model, to: directory, name: "error")
}

/// Requires this terminal's System Audio Recording permission. Not run by CI.
@Test(.enabled(if: ProcessInfo.processInfo.environment["TUNIC_TEST_AUDIO"] == "1")) @MainActor
func liveAudioThroughSwiftBindings() async throws {
    let model = TunicModel()
    defer { model.shutdown() }
    try await waitUntil("initial live snapshot") { model.snapshot != nil }
    let initial = try #require(model.snapshot)
    #expect(initial.audioError == nil)
    try #require(initial.acceptedChain != nil)
    let preset = try #require(initial.presets.first { $0.model == "HD650" })
    model.enqueue(.usePreset(id: preset.id))
    try await waitUntil("live preset selection") { model.snapshot?.presetId == preset.id }
    #expect(model.snapshot?.acceptedChain == model.snapshot?.desiredChain)
    let measurements = Task { await model.measureWhileVisible() }
    defer { measurements.cancel() }
    // Use another process because the system tap excludes the host itself.
    let sound = Process()
    sound.executableURL = URL(fileURLWithPath: "/usr/bin/afplay")
    sound.arguments = ["-v", "0.01", "/System/Library/Sounds/Glass.aiff"]
    try sound.run()
    defer { if sound.isRunning { sound.terminate() } }
    try await waitUntil("non-silent live telemetry") {
        model.telemetry.map { $0.leftRms > 0 || $0.rightRms > 0 } ?? false
    }
    #expect(model.telemetry?.spectrum.count == 256)
    try await waitUntil("numeric level readout") { model.levelText.hasPrefix("L ") }
    if let directory = ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"] {
        // This test already owns telemetry demand; don't mount another popup task.
        let renderer = ImageRenderer(content: ContentView(model: model)
            .background(Color(nsColor: .windowBackgroundColor)).environment(\.colorScheme, .dark))
        renderer.scale = 2
        let image = try #require(renderer.cgImage)
        let png = try #require(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]))
        try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("native-live.png"))
    }
    measurements.cancel()
    await measurements.value
    #expect(model.telemetry == nil)
    #expect(model.levelText == "No audio measurements")
    #expect(model.snapshot?.acceptedChain != nil)

    let reopened = Task { await model.measureWhileVisible() }
    defer { reopened.cancel() }
    try await waitUntil("readout after reopening") { model.levelText.hasPrefix("L ") }
    #expect(model.telemetry != nil)
    reopened.cancel()
    await reopened.value
    #expect(model.levelText == "No audio measurements")
}

@MainActor
private func render(_ model: TunicModel, to directory: String, name: String) async throws {
    _ = NSApplication.shared
    let host = NSHostingView(rootView: ContentView(model: model)
        .background(Color(nsColor: .windowBackgroundColor)).environment(\.colorScheme, .dark))
    let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 320, height: 360),
                          styleMask: [.borderless], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentView = host
    window.setContentSize(host.fittingSize)
    window.orderFrontRegardless()
    defer { window.close() }
    try await Task.sleep(for: .milliseconds(350))
    let capture = Process()
    capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    capture.arguments = ["-x", "-o", "-l", String(window.windowNumber),
                         directory + "/native-\(name).png", ]
    try capture.run()
    capture.waitUntilExit()
    #expect(capture.terminationStatus == 0)
}
