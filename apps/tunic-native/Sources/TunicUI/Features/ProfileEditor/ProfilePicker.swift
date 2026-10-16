import SwiftUI
import TunicEngine

struct ProfilePicker: View {
    let deviceName: String
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
                Image(systemName: "headphones")
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                VStack(spacing: 0) {
                    Text(deviceName)
                    Text(profileName ?? "Choose a profile")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                }
                .lineLimit(1)
                .frame(maxWidth: .infinity)
                Image(systemName: "chevron.up.chevron.down")
                    .font(.caption.weight(.semibold))
                    .accessibilityHidden(true)
            }
            .padding(.horizontal, 12)
            .frame(height: 36)
            .contentShape(.interaction, Capsule())
        }
        .menuIndicator(.hidden)
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive())
        .accessibilityLabel("Profile")
        .accessibilityValue("\(deviceName), \(profileName ?? "Choose a profile")")
        .help("Choose a profile for \(deviceName)")
        .frame(maxWidth: .infinity, alignment: .center)
    }
}
