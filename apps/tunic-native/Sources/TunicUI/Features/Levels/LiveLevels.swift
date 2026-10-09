import SwiftUI

// Observe readings here so telemetry doesn't invalidate the editing controls.
struct LiveLevels: View {
    enum Channel { case left, right }

    let model: TunicModel
    let channel: Channel

    var body: some View {
        CircularLevelMeter(rms: channel == .left ? model.telemetry?.leftRms : model.telemetry?.rightRms,
                           channel: channel)
            .accessibilityLabel(channel == .left ? "Left level" : "Right level")
    }
}

struct CircularLevelMeter: View {
    let rms: Float?
    let channel: LiveLevels.Channel
    @State private var envelope = LevelEnvelope()

    private var decibels: Double {
        20 * log10(max(Double(rms ?? 0), 0.001))
    }

    var body: some View {
        let fill = envelope.fill
        ZStack {
            Circle().stroke(.white.opacity(0.2), lineWidth: 1)
            Circle().trim(from: 0, to: fill)
                .stroke(.white.opacity(0.25 + 0.75 * fill),
                        style: StrokeStyle(lineWidth: 1, lineCap: .round))
                .rotationEffect(.degrees(-90))
                .scaleEffect(x: channel == .right ? -1 : 1, y: 1)
        }
        .frame(width: 28, height: 28)
        .accessibilityElement(children: .ignore)
        .accessibilityValue(rms == nil ? "No audio measurements" :
            decibels <= -60 ? "Silent" : "\(Int(decibels.rounded())) decibels full scale")
        .task(id: rms) {
            envelope.observe(rms, at: ProcessInfo.processInfo.systemUptime)
            do {
                while envelope.isDecaying {
                    try await Task.sleep(for: .milliseconds(16))
                    try Task.checkCancellation()
                    envelope.advance(to: ProcessInfo.processInfo.systemUptime)
                }
            } catch {
                // New readings replace this task; the envelope retains elapsed time.
            }
        }
        .onDisappear { envelope = LevelEnvelope() }
    }
}

/// Instant attack and a 100 ms exponential release of displayed dB height.
struct LevelEnvelope {
    private(set) var fill = 0.0
    private var target = 0.0
    private var lastTime: Double?

    var isDecaying: Bool { fill > target }

    mutating func observe(_ rms: Float?, at time: Double) {
        guard let rms else {
            self = LevelEnvelope()
            return
        }
        advance(to: time)
        let decibels = 20 * log10(max(Double(rms), 0.001))
        target = min(max((decibels + 60) / 60, 0), 1)
        fill = max(fill, target)
    }

    mutating func advance(to time: Double) {
        let elapsed = max(0, time - (lastTime ?? time))
        lastTime = time
        fill = target + (fill - target) * exp(-elapsed / 0.1)
        if fill - target < 0.001 { fill = target }
    }
}
