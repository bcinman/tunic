import SwiftUI
import TunicEngine

struct ProfileEditorView<Levels: View>: View {
    let state: StateSnapshot
    let send: (EngineCommand) -> Void
    var hoverChanged: (UInt32?) -> Void = { _ in }
    @ViewBuilder var levels: (LiveLevels.Channel) -> Levels

    var body: some View {
        Group {
            HStack(spacing: 8) {
                levels(.left)
                ProfilePicker(profileName: state.profileName, presets: state.presets, send: send)
                levels(.right)
            }
            .padding(.horizontal, 16)

            if !state.controls.isEmpty {
                VStack(spacing: 0) {
                    ForEach(state.controls, id: \.filter) { control in
                        GainControlRow(control: control) { gain in
                            send(.setControlGain(filter: control.filter, gain: gain))
                        }
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
            }
        }
    }
}
