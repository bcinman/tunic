import SwiftUI

struct AudioStatusView: View {
    let deviceName: String?

    var body: some View {
        if let deviceName {
            Text(deviceName)
                .font(.system(size: 13, weight: .medium))
                .lineLimit(1)
        }
    }
}
