import SwiftUI
import TunicEngine

struct ProfilePicker: View {
    let profileName: String?
    let presets: [Preset]
    let send: (EngineCommand) -> Void

    var body: some View {
        Menu {
            Button("Flat") { send(.useFlat) }
            ForEach(presets, id: \.id) { preset in
                Button("\(preset.brand) \(preset.model)") {
                    send(.usePreset(id: preset.id))
                }
            }
            Divider()
            Button("Clear selection") { send(.clearSelection) }
        } label: {
            HStack(spacing: 8) {
                Text(profileName ?? "Choose a profile")
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .contentShape(.interaction, Capsule())
        }
        .menuIndicator(.hidden)
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive())
        .accessibilityLabel("Profile")
        .frame(maxWidth: .infinity, alignment: .center)
    }
}
