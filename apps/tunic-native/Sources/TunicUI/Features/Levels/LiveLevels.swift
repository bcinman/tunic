import SwiftUI

// Observe readings here so telemetry doesn't invalidate the editing controls.
struct LiveLevels: View {
    let model: TunicModel

    var body: some View {
        Text(model.levelText)
            .font(.caption2.monospacedDigit())
            .foregroundStyle(.secondary)
    }
}
