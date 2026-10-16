import SwiftUI
import TunicEngine

struct ProfileEditorView<Levels: View>: View {
    let state: StateSnapshot
    let send: (EngineCommand) -> UInt64?
    var hoverChanged: (UInt32?) -> Void = { _ in }
    var dragChanged: (UInt32?) -> Void = { _ in }
    var addPreset: () -> Void = {}
    var addFlat: () -> Void = {}
    @ViewBuilder var levels: (LiveLevels.Channel) -> Levels

    var body: some View {
        Group {
            HStack(spacing: 8) {
                levels(.left)
                ProfilePicker(profileName: state.profileName, profiles: state.profiles, device: state.deviceId,
                              addPreset: addPreset, addFlat: addFlat, send: { _ = send($0) })
                levels(.right)
            }
            .padding(16)

            if let profile = state.selectedProfileId, !state.controls.isEmpty {
                Divider()
                    .padding(.horizontal, 8)
                    .padding(.bottom, 16)
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
                .id(state.selectedProfileId)
            }
        }
    }
}
