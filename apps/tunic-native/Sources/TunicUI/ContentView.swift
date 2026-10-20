import SwiftUI
import TunicEngine

public struct ContentView: View {
    private let model: TunicModel
    private let nowPlaying: NowPlayingModel?
    private let updates: AppUpdates?
    private let quit: () -> Void
    @State private var hoveredFilter: UInt32?
    @State private var draggedFilter: UInt32?
    @State private var creation: ProfileCreationDraft?

    public init(model: TunicModel, nowPlaying: NowPlayingModel? = nil,
                updates: AppUpdates? = nil, quit: @escaping () -> Void = {}) {
        self.model = model
        self.nowPlaying = nowPlaying
        self.updates = updates
        self.quit = quit
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                if let state = model.snapshot, creation == nil, state.selectedProfileId != nil {
                    ProfileHeaderView(state: state, send: model.enqueue,
                                      addPreset: { creation = ProfileCreationDraft(device: state.deviceId) },
                                      addFlat: {
                                          creation = ProfileCreationDraft(
                                              device: state.deviceId, source: .flat, name: "Custom")
                                      })
                } else {
                    AudioStatusView(deviceName: model.snapshot == nil ? "Connecting…" : model.snapshot?.deviceName)
                    Spacer()
                }
                HeaderActions(isProcessing: Binding(
                    get: { model.snapshot?.processingEnabled ?? true },
                    set: { model.enqueue(.setProcessingEnabled(enabled: $0)) }
                ), processingAvailable: model.snapshot != nil, nowPlaying: nowPlaying, updates: updates, quit: quit)
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.top, 12)
            if let state = model.snapshot {
                if creation != nil || state.selectedProfileId == nil {
                    ProfileCreationView(state: state, draft: Binding(
                        get: { creation ?? ProfileCreationDraft(device: state.deviceId) },
                        set: { if model.snapshot?.deviceId == state.deviceId { creation = $0 } }),
                        cancel: state.selectedProfileId == nil ? nil : { creation = nil },
                        submit: { submit(device: state.deviceId) })
                    .id(state.deviceId)
                } else {
                    VisualizerView(model: model, maximumFrequency: min(20_000, state.sampleRate * 0.499),
                                   hoveredFilter: draggedFilter ?? hoveredFilter)
                    ProfileEditorView(state: state, send: model.enqueue,
                                      hoverChanged: { hoveredFilter = $0 },
                                      dragChanged: { draggedFilter = $0 })
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
            if let nowPlaying { NowPlayingView(model: nowPlaying) }
        }
        .padding(.bottom, 12)
        .frame(width: 320)
        .preferredColorScheme(.dark)
        .task { await model.measureWhileVisible() }
        .onAppear { nowPlaying?.setVisible(true) }
        .onChange(of: model.snapshot?.deviceId) {
            creation = nil
        }
        .onChange(of: model.snapshot?.revision) {
            if let state = model.snapshot, creation?.reconcile(state) == true { creation = nil }
        }
        .onChange(of: model.snapshot?.selectedProfileId) {
            hoveredFilter = nil
            draggedFilter = nil
        }
        .onDisappear {
            nowPlaying?.setVisible(false)
            hoveredFilter = nil
            draggedFilter = nil
        }
    }

    private func submit(device: String) {
        guard model.snapshot?.canCreateProfile == true, model.snapshot?.deviceId == device, var draft = creation,
              draft.device == device, draft.receipt == nil, let source = draft.source else { return }
        let name = draft.name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return }
        draft.previousSelection = model.snapshot?.selectedProfileId
        draft.receipt = model.enqueue(.createProfile(device: device, name: name, source: source))
        creation = draft
    }
}

#Preview(traits: .sizeThatFitsLayout) {
    ContentView(model: TunicModel(connectAudio: false))
}
