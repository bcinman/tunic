import SwiftUI
import TunicEngine

struct ProfilePicker: View {
    let deviceName: String
    let profileName: String?
    let selectedProfileId: String?
    let profiles: [ProfileSummary]
    let device: String
    var processingEnabled = true
    var available = true
    var addPreset: () -> Void = {}
    var addFlat: () -> Void = {}
    let send: (EngineCommand) -> Void

    var body: some View {
        Menu {
            Toggle("Disabled", isOn: Binding(
                get: { !processingEnabled },
                set: { selected in
                    if selected { send(.disableProcessing(device: device)) }
                }
            ))
            .disabled(!available)
            Divider()
            ForEach(profiles, id: \.id) { profile in
                Toggle(profile.name, isOn: Binding(
                    get: { profile.id == selectedProfileId },
                    set: { selected in
                        if selected {
                            send(.selectProfile(device: device, profile: profile.id))
                        }
                    }
                ))
            }
            Divider()
            Button("Add from preset…", systemImage: "slider.horizontal.3", action: addPreset)
            Button("Start flat…", systemImage: "line.3.horizontal", action: addFlat)
        } label: {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(deviceName)
                    HStack(spacing: 5) {
                        Text(processingEnabled ? profileName ?? "Choose a profile" : "Disabled")
                        Image(systemName: "chevron.up.chevron.down")
                            .font(.system(size: 9, weight: .semibold))
                            .accessibilityHidden(true)
                    }
                        .foregroundStyle(.primary.opacity(0.65))
                }
                .font(.system(size: 13, weight: .medium))
                .lineLimit(1)
            }
            .frame(height: 36)
            .contentShape(Rectangle())
        }
        .menuIndicator(.hidden)
        .buttonStyle(.plain)
        .accessibilityLabel("Profile")
        .accessibilityValue("\(deviceName), \(processingEnabled ? profileName ?? "Choose a profile" : "Disabled")")
        .help("Choose a profile for \(deviceName)")
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
