import SwiftUI
import TunicEngine

struct ProfilePicker: View {
    let profileName: String?
    let profiles: [ProfileSummary]
    let device: String
    var addPreset: () -> Void = {}
    var addFlat: () -> Void = {}
    let send: (EngineCommand) -> Void

    var body: some View {
        Menu {
            ForEach(profiles, id: \.id) { profile in
                Button {
                    send(.selectProfile(device: device, profile: profile.id))
                } label: {
                    if profile.name == profileName {
                        Label(profile.name, systemImage: "checkmark")
                    } else {
                        Text(profile.name)
                    }
                }
            }
            Divider()
            Button("Add from preset…", action: addPreset)
            Button("Start flat…", action: addFlat)
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
