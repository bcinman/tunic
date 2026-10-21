import AppKit
import Observation

@MainActor @Observable
public final class NowPlayingModel {
    private(set) var playback = PlaybackState()
    private(set) var artwork: NSImage?
    private(set) var connecting: MusicPlayer?
    private let readers: [MusicPlayer: any PlayerReading]
    private let runningProcess: (MusicPlayer) -> pid_t?
    private var observers: [(NotificationCenter, NSObjectProtocol)] = []
    private var refreshTask: Task<Void, Never>?
    private var artworkTask: Task<Void, Never>?
    private var generation = 0
    private var pendingEvent: MusicPlayer?
    private var visible = false
    private var stopped = false
    private var artworkTrack: PlayingTrack?
    private var artworkPlayer: MusicPlayer?

    public init() {
        readers = [.music: PlayerReader(), .spotify: PlayerReader()]
        runningProcess = Self.pid
        for player in MusicPlayer.allCases {
            observe(DistributedNotificationCenter.default(), name: Notification.Name(player.notification)) {
                $0.refresh(event: player)
            }
        }
        let workspace = NSWorkspace.shared.notificationCenter
        for name in [NSWorkspace.didLaunchApplicationNotification, NSWorkspace.didTerminateApplicationNotification,
                     NSWorkspace.didWakeNotification, ] {
            observe(workspace, name: name) { $0.refresh() }
        }
        observe(NotificationCenter.default, name: NSApplication.didBecomeActiveNotification) { $0.refresh() }
        refresh()
    }

    /// Inert state for previews; never observes players or requests permission.
    init(playback: PlaybackState, artwork: NSImage? = nil) {
        readers = [:]
        runningProcess = { _ in nil }
        self.playback = playback
        self.artwork = artwork
        stopped = true
    }

    init(readers: [MusicPlayer: any PlayerReading], runningProcess: @escaping (MusicPlayer) -> pid_t?) {
        self.readers = readers
        self.runningProcess = runningProcess
    }

    private func observe(_ center: NotificationCenter, name: Notification.Name,
                         action: @escaping @MainActor (NowPlayingModel) -> Void) {
        let token = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { if let self { action(self) } }
        }
        observers.append((center, token))
    }

    func setVisible(_ value: Bool) {
        visible = value
        if value { refresh() } else { artworkTask?.cancel(); artworkTask = nil }
    }

    func connect(_ player: MusicPlayer) {
        guard !stopped, connecting == nil, Bundle.main.bundleURL.pathExtension == "app" else { return }
        if playback.players[player] == .denied {
            let settings = "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation"
            NSWorkspace.shared.open(URL(string: settings)!)
            return
        }
        guard let pid = runningProcess(player), let reader = readers[player] else { return }
        connecting = player
        refreshTask?.cancel()
        generation += 1
        refreshTask = Task {
            _ = await reader.read(player, pid: pid, authorize: true)
            guard !Task.isCancelled else { return }
            connecting = nil
            refreshTask = nil
            refresh()
        }
    }

    func refresh(event: MusicPlayer? = nil) {
        guard !stopped else { return }
        generation += 1
        if let event { pendingEvent = event }
        // Remove exited sources immediately, including while another player is awaiting consent.
        let absent = MusicPlayer.allCases.filter { runningProcess($0) == nil }
        playback.receive(Dictionary(uniqueKeysWithValues: absent.map { ($0, .notRunning) }))
        updateArtwork()
        guard refreshTask == nil else { return }
        refreshTask = Task {
            repeat {
                let revision = generation
                let event = pendingEvent
                pendingEvent = nil
                var observations: [MusicPlayer: PlayerState] = [:]
                // Read both before publishing so the initial tie-break never depends on response order.
                for player in MusicPlayer.allCases {
                    guard let pid = runningProcess(player), let reader = readers[player] else {
                        observations[player] = .notRunning
                        continue
                    }
                    observations[player] = await reader.read(player, pid: pid, authorize: false)
                }
                guard !Task.isCancelled else { return }
                if revision == generation {
                    playback.receive(observations, playbackEvent: event)
                    updateArtwork()
                    break
                }
                if pendingEvent == nil { pendingEvent = event }
            } while !Task.isCancelled
            refreshTask = nil
        }
    }

    private func updateArtwork() {
        let player = playback.selected
        let track = player.flatMap { playback.players[$0]?.track }
        if artworkTrack != track || artworkPlayer != player {
            artworkTask?.cancel()
            artworkTask = nil
            artwork = nil
            artworkTrack = track
            artworkPlayer = player
        }
        guard visible, artwork == nil, artworkTask == nil, let player, let track,
              let pid = runningProcess(player), let reader = readers[player] else { return }
        artworkTask = Task {
            let data = await reader.artwork(player, pid: pid, track: track)
            guard !Task.isCancelled, artworkTrack == track, artworkPlayer == player else { return }
            artwork = data.flatMap(NSImage.init(data:))
            artworkTask = nil
        }
    }

    private static func pid(_ player: MusicPlayer) -> pid_t? {
        NSRunningApplication.runningApplications(withBundleIdentifier: player.rawValue)
            .first(where: { !$0.isTerminated })?.processIdentifier
    }

    public func shutdown() {
        stopped = true
        refreshTask?.cancel()
        artworkTask?.cancel()
        for (center, token) in observers { center.removeObserver(token) }
        observers.removeAll()
    }

    isolated deinit {
        refreshTask?.cancel()
        artworkTask?.cancel()
        for (center, token) in observers { center.removeObserver(token) }
    }
}
