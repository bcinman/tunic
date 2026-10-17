import SwiftUI
import TunicEngine

struct ProfileHeaderView: View {
    let state: StateSnapshot
    let send: (EngineCommand) -> UInt64?
    var addPreset: () -> Void = {}
    var addFlat: () -> Void = {}

    var body: some View {
        ProfilePicker(deviceName: state.deviceName, profileName: state.profileName,
                      selectedProfileId: state.selectedProfileId,
                      profiles: state.profiles, device: state.deviceId,
                      addPreset: addPreset, addFlat: addFlat, send: { _ = send($0) })
    }
}

struct ProfileEditorView: View {
    let state: StateSnapshot
    let send: (EngineCommand) -> UInt64?
    var hoverChanged: (UInt32?) -> Void = { _ in }
    var dragChanged: (UInt32?) -> Void = { _ in }

    var body: some View {
        Group {
            if let profile = state.selectedProfileId, !state.controls.isEmpty {
                VStack(spacing: 0) {
                    ForEach(state.controls, id: \.filter) { control in
                        GainControlRow(profile: profile, control: control,
                                       processedCommand: state.processedCommand, send: send,
                                       dragChanged: { dragChanged($0 ? control.filter : nil) })
                        .padding(.top, control.filter == state.controls.first?.filter ? 0 : 8)
                        .padding(.bottom, control.filter == state.controls.last?.filter ? 0 : 8)
                        .contentShape(Rectangle())
                        .onHover { inside in
                            if inside { hoverChanged(control.filter) }
                        }
                    }
                }
                .contentShape(Rectangle())
                .onHover { inside in
                    if !inside { hoverChanged(nil) }
                }
                .padding(.top, 16)
                .id(state.selectedProfileId)
            }
        }
    }
}
