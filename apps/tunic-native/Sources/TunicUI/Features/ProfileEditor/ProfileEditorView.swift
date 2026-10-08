import SwiftUI
import TunicEngine

struct ProfileEditorView<Levels: View>: View {
    let state: StateSnapshot
    let send: (EngineCommand) -> Void
    @ViewBuilder var levels: () -> Levels

    var body: some View {
        Group {
            ProfilePicker(profileName: state.profileName, presets: state.presets, send: send)

            ForEach(state.controls, id: \.filter) { control in
                GainControlRow(control: control) { gain in
                    send(.setControlGain(filter: control.filter, gain: gain))
                }
            }

            HStack {
                levels()
                Spacer()
                Button("Reset") { send(.resetDraft) }
                    .disabled(!state.hasDraft)
                Button("Save") { send(.saveDraft) }
                    .disabled(!state.hasDraft)
            }
            .controlSize(.small)
            .padding(.horizontal, 12)
        }
    }
}
