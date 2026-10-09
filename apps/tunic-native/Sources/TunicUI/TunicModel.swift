import Foundation
import Observation
import TunicEngine

/// Presentation only. Rust owns profile edits, persistence, and audio lifecycle.
@MainActor @Observable
public final class TunicModel {
    public private(set) var snapshot: StateSnapshot?
    public private(set) var response: [Double] = []
    public private(set) var telemetry: TelemetryFrame?
    public private(set) var error: String?

    private let engine: Engine?
    private var updates: Task<Void, Never>?

    public init(connectAudio: Bool = true) {
        do {
            var databasePath: String?
            if connectAudio {
                let directory = try FileManager.default.url(
                    for: .applicationSupportDirectory, in: .userDomainMask,
                    appropriateFor: nil, create: true
                ).appendingPathComponent("Tunic", isDirectory: true)
                try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
                databasePath = directory.appendingPathComponent("session.sqlite3").path
            }
            let engine = try Engine(connectAudio: connectAudio, databasePath: databasePath)
            self.engine = engine
            updates = Task { [weak self] in
                for await _ in engine.updates() {
                    guard !Task.isCancelled else { break }
                    self?.refresh(from: engine)
                }
            }
        } catch {
            engine = nil
            self.error = String(describing: error)
        }
    }

    isolated deinit { updates?.cancel() }

    public func enqueue(_ command: EngineCommand) {
        guard let engine else { return }
        do {
            try engine.enqueue(command: command)
            error = nil
        } catch {
            self.error = String(describing: error)
        }
    }

    private func refresh(from engine: Engine) {
        guard let next = engine.snapshot(), next.revision != snapshot?.revision else { return }
        if next.audioGeneration != snapshot?.audioGeneration {
            telemetry = nil
        }
        snapshot = next
        let maximum = min(20_000, next.sampleRate * 0.499)
        let frequencies = (0..<128).map { 20 * pow(maximum / 20, Double($0) / 127) }
        do {
            response = try frequencyResponse(
                chain: next.desiredChain, sampleRate: next.sampleRate, frequencies: frequencies
            )
        } catch {
            response = []
            self.error = String(describing: error)
        }
    }

    /// SwiftUI cancels this task when the popup disappears. Audio keeps running.
    public func measureWhileVisible() async {
        guard let engine else { return }
        do {
            try engine.setTelemetryEnabled(enabled: true)
            defer {
                try? engine.setTelemetryEnabled(enabled: false)
                telemetry = nil
            }
            var generation: UInt64?
            while !Task.isCancelled {
                let batch = engine.pollTelemetry()
                if generation != batch.audioGeneration {
                    telemetry = nil
                }
                generation = batch.audioGeneration
                if let frame = batch.frames.last {
                    telemetry = frame
                }
                try await Task.sleep(for: .milliseconds(33))
            }
        } catch is CancellationError {
            // Closing the popup ends display demand, not the audio route.
        } catch {
            self.error = String(describing: error)
        }
    }

    public func shutdown() {
        updates?.cancel()
        updates = nil
        engine?.shutdown()
    }
}
