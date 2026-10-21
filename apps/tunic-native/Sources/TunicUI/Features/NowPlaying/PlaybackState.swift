import Foundation

enum MusicPlayer: String, CaseIterable, Sendable {
    case music = "com.apple.Music"
    case spotify = "com.spotify.client"

    var name: String { self == .music ? "Apple Music" : "Spotify" }
    var notification: String {
        self == .music ? "com.apple.Music.playerInfo" : "com.spotify.client.PlaybackStateChanged"
    }
}

struct PlayingTrack: Equatable, Sendable {
    let identifier: String
    let title: String
    let artist: String
    let album: String
}

enum PlayerState: Equatable, Sendable {
    case notRunning, needsAuthorization, denied, unavailable
    case stopped
    case playing(PlayingTrack)
    case paused(PlayingTrack)

    var track: PlayingTrack? {
        switch self {
        case .playing(let track), .paused(let track): track
        default: nil
        }
    }

    var isPlaying: Bool {
        if case .playing = self { return true }
        return false
    }

    var description: String {
        switch self {
        case .notRunning: "Not running"
        case .needsAuthorization: "Connect"
        case .denied: "Allow in System Settings"
        case .unavailable: "Unavailable — retry"
        case .stopped: "Connected"
        case .playing: "Playing"
        case .paused: "Paused"
        }
    }
}

/// Selection depends on playback transitions, never artwork completion or read completion order.
struct PlaybackState {
    private(set) var players: [MusicPlayer: PlayerState] = [:]
    private(set) var selected: MusicPlayer?

    mutating func receive(_ observations: [MusicPlayer: PlayerState], playbackEvent: MusicPlayer? = nil) {
        let previous = players
        players.merge(observations) { _, new in new }
        if let source = playbackEvent, players[source]?.isPlaying == true,
           previous[source]?.isPlaying != true {
            selected = source
        }
        if let selected, players[selected]?.isPlaying == true { return }
        if let playing = MusicPlayer.allCases.first(where: { players[$0]?.isPlaying == true }) {
            selected = playing
        } else if let selected, players[selected]?.track != nil {
            return
        } else {
            selected = MusicPlayer.allCases.first { players[$0]?.track != nil }
        }
    }
}
