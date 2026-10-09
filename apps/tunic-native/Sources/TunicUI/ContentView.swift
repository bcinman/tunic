import SwiftUI

public struct ContentView: View {
    private let model: TunicModel
    private let quit: () -> Void
    @State private var hoveredFilter: UInt32?

    public init(model: TunicModel, quit: @escaping () -> Void = {}) {
        self.model = model
        self.quit = quit
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            AudioStatusView(deviceName: model.snapshot?.deviceName,
                            isProcessing: model.snapshot?.acceptedChain != nil)

            if let state = model.snapshot {
                VisualizerView(model: model, maximumFrequency: min(20_000, state.sampleRate * 0.499),
                               hoveredFilter: hoveredFilter)
                ProfileEditorView(state: state, send: model.enqueue,
                                  hoverChanged: { hoveredFilter = $0 },
                                  levels: { LiveLevels(model: model, channel: $0) })
            }

            if let error = model.error ?? model.snapshot?.actionError {
                Text(error).font(.caption).foregroundStyle(.red)
                    .padding(.horizontal, 12)
            }
            if let error = model.snapshot?.audioError {
                Text(error).font(.caption).foregroundStyle(.orange)
                    .padding(.horizontal, 12)
            }
            Button("Quit Tunic", systemImage: "power", action: quit)
                .labelStyle(.iconOnly)
                .help("Quit Tunic")
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 12)
                .padding(.bottom, 12)
        }
        .frame(width: 320)
        .task { await model.measureWhileVisible() }
        .onChange(of: model.snapshot?.presetId) { hoveredFilter = nil }
        .onDisappear { hoveredFilter = nil }
    }
}

#Preview {
    ContentView(model: TunicModel(connectAudio: false))
}
