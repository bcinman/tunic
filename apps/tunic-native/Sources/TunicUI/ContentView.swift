import SwiftUI
import TunicEngine

public struct ContentView: View {
    private let model: TunicModel
    private let quit: () -> Void
    @State private var hoveredFilter: UInt32?
    @State private var draggedFilter: UInt32?
    @State private var creation: ProfileCreationDraft?

    public init(model: TunicModel, quit: @escaping () -> Void = {}) {
        self.model = model
        self.quit = quit
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let state = model.snapshot {
                if creation != nil || state.selectedProfileId == nil {
                    ProfileCreationView(state: state, draft: Binding(
                        get: { creation ?? ProfileCreationDraft(device: state.deviceId) },
                        set: { if model.snapshot?.deviceId == state.deviceId { creation = $0 } }),
                        cancel: state.selectedProfileId == nil ? nil : { creation = nil },
                        submit: { submit(device: state.deviceId) })
                    .id(state.deviceId)
                } else {
                    ProfileHeaderView(state: state, send: model.enqueue,
                                      addPreset: { creation = ProfileCreationDraft(device: state.deviceId) },
                                      addFlat: {
                                          creation = ProfileCreationDraft(
                                              device: state.deviceId, source: .flat, name: "Custom")
                                      },
                                      levels: { LiveLevels(model: model, channel: $0) })
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
            HStack {
                AudioStatusView(deviceName: model.snapshot == nil ? "Connecting…" :
                                    (creation != nil || model.snapshot?.selectedProfileId == nil
                                        ? model.snapshot?.deviceName : nil),
                                isProcessing: Binding(
                                    get: { model.snapshot?.processingEnabled ?? true },
                                    set: { model.enqueue(.setProcessingEnabled(enabled: $0)) }
                                ))
                    .disabled(model.snapshot == nil)
                Spacer()
                SettingsMenu(quit: quit)
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.top, 12)
        }
        .padding(.bottom, 12)
        .frame(width: 320)
        .background(.black.opacity(0.1))
        .preferredColorScheme(.dark)
        .task { await model.measureWhileVisible() }
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
