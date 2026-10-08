import SwiftUI

public struct ContentView: View {
    private let model: TunicModel
    private let quit: () -> Void

    public init(model: TunicModel, quit: @escaping () -> Void = {}) {
        self.model = model
        self.quit = quit
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            AudioStatusView(deviceName: model.snapshot?.deviceName,
                            isProcessing: model.snapshot?.acceptedChain != nil)

            if let state = model.snapshot {
                VisualizerView(model: model, maximumFrequency: min(20_000, state.sampleRate * 0.499))
                ProfileEditorView(state: state, send: model.enqueue) {
                    LiveLevels(model: model)
                }
            }

            if let error = model.error ?? model.snapshot?.actionError {
                Text(error).font(.caption).foregroundStyle(.red)
                    .padding(.horizontal, 12)
            }
            if let error = model.snapshot?.audioError {
                Text(error).font(.caption).foregroundStyle(.orange)
                    .padding(.horizontal, 12)
            }
            Divider()
                .padding(.horizontal, 12)
            Button("Quit Tunic", action: quit)
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 12)
                .padding(.bottom, 12)
        }
        .frame(width: 320)
        .task { await model.measureWhileVisible() }
    }
}

#Preview {
    ContentView(model: TunicModel(connectAudio: false))
}
