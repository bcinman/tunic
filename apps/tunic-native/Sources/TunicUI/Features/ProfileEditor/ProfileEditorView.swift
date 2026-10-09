import SwiftUI
import TunicEngine

struct ProfileEditorView<Levels: View>: View {
    let state: StateSnapshot
    let send: (EngineCommand) -> Void
    @ViewBuilder var levels: (LiveLevels.Channel) -> Levels

    var body: some View {
        Group {
            HStack(spacing: 8) {
                levels(.left)
                ProfilePicker(profileName: state.profileName, presets: state.presets, send: send)
                levels(.right)
            }
            .padding(.horizontal, 16)

            ForEach(state.controls, id: \.filter) { control in
                GainControlRow(control: control) { gain in
                    send(.setControlGain(filter: control.filter, gain: gain))
                }
            }
        }
    }
}
