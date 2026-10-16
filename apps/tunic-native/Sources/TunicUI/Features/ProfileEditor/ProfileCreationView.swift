import SwiftUI
import TunicEngine

/// Local presentation state only; searching never changes the engine or audio.
struct ProfileCreationDraft {
    var device: String?
    var query = ""
    var source: ProfileSource?
    var name = ""
    var receipt: UInt64?
    var previousSelection: String?

    static func suggestedName(_ preset: Preset) -> String {
        [preset.brand, preset.model, preset.variant ?? ""].filter { !$0.isEmpty }.joined(separator: " ")
    }

    static func matches(_ preset: Preset, query: String) -> Bool {
        func normalized(_ text: String) -> String {
            text.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "en_US_POSIX"))
                .filter { $0.isLetter || $0.isNumber }
        }
        let haystack = normalized(suggestedName(preset))
        return query.split(whereSeparator: { $0.isWhitespace }).allSatisfy { haystack.contains(normalized(String($0))) }
    }

    mutating func reconcile(_ state: StateSnapshot) -> Bool {
        guard let receipt, state.processedCommand >= receipt else { return false }
        self.receipt = nil
        // Persistence can succeed even when publishing the new chain to audio fails.
        return state.selectedProfileId != previousSelection && state.selectedProfileId != nil
    }
}

struct ProfileCreationView: View {
    let state: StateSnapshot
    @Binding var draft: ProfileCreationDraft
    let cancel: (() -> Void)?
    let submit: () -> Void

    private var results: [Preset] {
        state.presets.filter { ProfileCreationDraft.matches($0, query: draft.query) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Find your headphones").font(.headline)
                Spacer()
                if let cancel { Button("Cancel", action: cancel).font(.caption) }
            }
            TextField("Search brand or model", text: $draft.query)
                .textFieldStyle(.roundedBorder)
                .onChange(of: draft.query) {
                    draft.source = nil
                    draft.name = ""
                }
            if !draft.query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                if results.isEmpty {
                    Text("No headphones found").foregroundStyle(.secondary)
                    Button("Clear search") { draft.query = "" }
                } else {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 4) {
                            ForEach(results, id: \.id) { preset in
                                Button {
                                    draft.source = .preset(id: preset.id)
                                    draft.name = ProfileCreationDraft.suggestedName(preset)
                                } label: {
                                    HStack {
                                        Text(ProfileCreationDraft.suggestedName(preset))
                                            .frame(maxWidth: .infinity, alignment: .leading)
                                        if case .preset(let presetId) = draft.source, presetId == preset.id {
                                            Image(systemName: "checkmark")
                                        }
                                    }.padding(8).contentShape(Rectangle())
                                }.buttonStyle(.plain)
                            }
                        }
                    }.frame(height: 156)
                }
            }
            Button("Start with a flat EQ") {
                draft.source = .flat
                draft.name = "Custom"
            }.buttonStyle(.link)
            if draft.source != nil {
                if case .flat = draft.source {
                    Text("Neutral EQ · five editable bands").font(.caption).foregroundStyle(.secondary)
                }
                TextField("Profile name", text: $draft.name).textFieldStyle(.roundedBorder)
                    .accessibilityLabel("Profile name")
                if nameExists {
                    Text("This device already has a profile with that name.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Button(draft.receipt == nil ? "Add profile" : "Adding…", action: submit)
                    .buttonStyle(.borderedProminent)
                    .disabled(draft.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || nameExists)
                    .disabled(!state.canCreateProfile)
                    .help(state.canCreateProfile ? "Create and select this profile" : "Waiting for an audio device")
            }
        }
        .disabled(draft.receipt != nil)
        .padding(16)
    }

    private var nameExists: Bool {
        let name = draft.name.trimmingCharacters(in: .whitespacesAndNewlines)
        return state.profiles.contains { $0.name == name }
    }
}
