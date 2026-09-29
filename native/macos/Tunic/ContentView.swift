import SwiftUI

struct ContentView: View {
    @State private var selectedProfile = "Sennheiser HD650"

    var body: some View {
        Color.clear
            // Keep the empty window visible until it has real content.
            .frame(height: 320)
            .frame(minWidth: 480, idealWidth: 748, maxWidth: 748)
            .fixedSize(horizontal: false, vertical: true)
            .toolbarBackground(.hidden, for: .windowToolbar)
            .toolbar {
                ToolbarItem(placement: .navigation) {
                    Label("External Headphones", systemImage: "headphones")
                }
                ToolbarItem(placement: .navigation) {
                    // Static choices only; selection does not change audio.
                    Picker("Profile", selection: $selectedProfile) {
                        Text("Sennheiser HD650").tag("Sennheiser HD650")
                        Text("Flat").tag("Flat")
                    }
                    .labelsHidden()
                    .pickerStyle(.menu)
                }
            }
    }
}

#Preview {
    ContentView()
}
