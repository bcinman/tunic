import SwiftUI
import TunicEngine

/// Sensitivity to a gain change, including when the filter is currently flat.
func gainInfluence(filter: Filter, sampleRate: Double, frequencies: [Double]) throws -> [Double] {
    var probe = filter
    let step = filter.gain > 0 ? -0.1 : 0.1
    probe.gain += step
    let original = try frequencyResponse(chain: Chain(preamp: 0, filters: [filter]),
                                         sampleRate: sampleRate, frequencies: frequencies)
    let adjusted = try frequencyResponse(chain: Chain(preamp: 0, filters: [probe]),
                                         sampleRate: sampleRate, frequencies: frequencies)
    return zip(original, adjusted).map { min(1, abs(($1 - $0) / step)) }
}

struct GainResponseView: View {
    let response: [Double]
    let influence: [Double]
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        GeometryReader { geometry in
            let points = response.enumerated().map { index, gain in
                CGPoint(x: geometry.size.width * Double(index) / Double(max(1, response.count - 1)),
                        y: geometry.size.height * (0.5 - gain / 48))
            }
            let gradient = LinearGradient(stops: influence.enumerated().map { index, weight in
                Gradient.Stop(color: Color.white.opacity(weight),
                              location: Double(index) / Double(max(1, influence.count - 1)))
            }, startPoint: .leading, endPoint: .trailing)
            let curve = Path { path in
                path.addLines(points)
            }
            let fill = Path { path in
                guard let first = points.first, let last = points.last else { return }
                path.move(to: CGPoint(x: first.x, y: geometry.size.height / 2))
                for point in points { path.addLine(to: point) }
                path.addLine(to: CGPoint(x: last.x, y: geometry.size.height / 2))
                path.closeSubpath()
            }
            fill.fill(gradient).opacity(0.12)
            Path { path in
                path.move(to: CGPoint(x: 0, y: geometry.size.height / 2))
                path.addLine(to: CGPoint(x: geometry.size.width, y: geometry.size.height / 2))
            }
            .stroke(.white.opacity(0.12), lineWidth: 1 / displayScale)
            curve.stroke(.white.opacity(0.16), lineWidth: 1)
            curve.stroke(gradient, style: StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
        }
        .clipped()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}
