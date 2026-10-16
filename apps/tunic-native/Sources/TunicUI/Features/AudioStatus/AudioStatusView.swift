import SwiftUI

struct AudioStatusView: View {
    let deviceName: String?

    var body: some View {
        if let deviceName {
            Label(deviceName, systemImage: "headphones")
                .lineLimit(1)
        }
    }
}
