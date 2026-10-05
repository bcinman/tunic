import AppKit
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
    )])
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
    try render(model, to: directory, name: "empty")
    let preset = try #require(model.snapshot?.presets.first { $0.model == "HD650" })
    model.enqueue(.usePreset(id: preset.id))
    try await waitUntil { model.snapshot?.presetId == preset.id }
    let control = try #require(model.snapshot?.controls.first)
    model.enqueue(.setControlGain(filter: control.filter, gain: 3))
    try await waitUntil { model.snapshot?.hasDraft == true }
    try render(model, to: directory, name: "edited")
    model.enqueue(.usePreset(id: "missing"))
    try await waitUntil { model.snapshot?.actionError != nil }
    try render(model, to: directory, name: "error")
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
    if let directory = ProcessInfo.processInfo.environment["TUNIC_SCREENSHOTS"] {
        try render(model, to: directory, name: "live")
    }
    measurements.cancel()
    await measurements.value
    #expect(model.telemetry == nil)
    #expect(model.snapshot?.acceptedChain != nil)
}

@MainActor
private func render(_ model: TunicModel, to directory: String, name: String) throws {
    let renderer = ImageRenderer(content: ContentView(model: model)
        .background(Color(nsColor: .windowBackgroundColor)).environment(\.colorScheme, .dark))
    renderer.scale = 2
    let image = try #require(renderer.cgImage)
    let png = try #require(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]))
    try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("native-\(name).png"))
}
