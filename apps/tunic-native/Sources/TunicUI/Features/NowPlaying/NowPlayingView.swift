import SwiftUI

struct NowPlayingView: View {
    let model: NowPlayingModel

    var body: some View {
        if let player = model.playback.selected, let state = model.playback.players[player], let track = state.track {
            Divider()
                .padding(.horizontal, 12)
                .padding(.top, 12)
                .accessibilityHidden(true)
            HStack(spacing: 10) {
                Group {
                    if let artwork = model.artwork {
                        Image(nsImage: artwork).resizable().scaledToFill()
                    } else {
                        Image(systemName: "music.note")
                            .font(.system(size: 16)).foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                            .background(.white.opacity(0.06))
                    }
                }
                .frame(width: 32, height: 32)
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(track.title)
                    Text(track.artist.isEmpty ? player.name : track.artist)
                        .foregroundStyle(.primary.opacity(0.65))
                }
                .font(.system(size: 13, weight: .medium))
                .lineLimit(1)
                Spacer(minLength: 0)
                VStack(alignment: .trailing, spacing: 3) {
                    Text(player == .music ? "Music" : "Spotify")
                    if !state.isPlaying { Text("Paused") }
                }
                .font(.system(size: 9)).foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.top, 12)
            .help([track.title, track.artist, track.album, player.name].filter { !$0.isEmpty }.joined(separator: " — "))
            .accessibilityElement(children: .combine)
        }
    }
}

struct NowPlayingSettings: View {
    let model: NowPlayingModel

    var body: some View {
        Menu("Now Playing") {
            ForEach(MusicPlayer.allCases, id: \.self) { player in
                let state = model.playback.players[player] ?? .notRunning
                Button("\(player.name): \(model.connecting == player ? "Connecting…" : state.description)") {
                    model.connect(player)
                }
                .disabled(state == .notRunning || model.connecting != nil
                          || Bundle.main.bundleURL.pathExtension != "app")
            }
            if Bundle.main.bundleURL.pathExtension != "app" {
                Text("Requires the Tunic app bundle")
            } else {
                Text("Open a player, then connect to show its current song.")
            }
        }
    }
}
