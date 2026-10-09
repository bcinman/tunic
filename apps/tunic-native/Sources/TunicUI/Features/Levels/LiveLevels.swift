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

    private var decibels: Double {
        20 * log10(Double(max(rms ?? 0, 0.001)))
    }

    var body: some View {
        let fill = min(max((decibels + 60) / 60, 0), 1)
        ZStack {
            Circle().stroke(.white.opacity(0.2), lineWidth: 1)
            Circle().trim(from: 0, to: fill)
                .stroke(.white, style: StrokeStyle(lineWidth: 1, lineCap: .round))
                .rotationEffect(.degrees(-90))
                .scaleEffect(x: channel == .right ? -1 : 1, y: 1)
        }
        .frame(width: 28, height: 28)
        .accessibilityElement(children: .ignore)
        .accessibilityValue(rms == nil ? "No audio measurements" :
            decibels <= -60 ? "Silent" : "\(Int(decibels.rounded())) decibels full scale")
    }
}
