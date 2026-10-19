import AppKit
import MetalKit
import SwiftUI
import Testing
@testable import TunicUI

private let musicTrack = PlayingTrack(identifier: "music:1", title: "Everything in Its Right Place",
                                      artist: "Radiohead", album: "Kid A")
private let spotifyTrack = PlayingTrack(identifier: "spotify:2", title: "A Walk", artist: "Tycho", album: "Dive")

@Test
func nowPlayingSelectionFollowsPlaybackNotMetadataRefreshes() {
    var state = PlaybackState()
    state.receive([.spotify: .playing(spotifyTrack), .music: .playing(musicTrack)])
    #expect(state.selected == .music) // Deterministic startup tie.
    state.receive([.spotify: .playing(spotifyTrack)], playbackEvent: .spotify)
    #expect(state.selected == .music) // An already-playing source does not steal selection.
    state.receive([.spotify: .paused(spotifyTrack)], playbackEvent: .spotify)
    state.receive([.spotify: .playing(spotifyTrack)], playbackEvent: .spotify)
    #expect(state.selected == .spotify) // A genuine play transition does.
    state.receive([.music: .playing(musicTrack)])
    #expect(state.selected == .spotify)
    state.receive([.spotify: .paused(spotifyTrack)])
    #expect(state.selected == .music) // Playing beats paused.
    state.receive([.music: .paused(musicTrack)])
    #expect(state.selected == .music) // Keep paused context.
    state.receive([.music: .notRunning])
    #expect(state.selected == .spotify)
    state.receive([.spotify: .stopped])
    #expect(state.selected == nil)
    for unavailable in [PlayerState.needsAuthorization, .denied, .unavailable, .notRunning] {
        state.receive([.music: unavailable])
        #expect(state.selected == nil)
        #expect(state.players[.music] == unavailable)
    }
}

private actor DelayedPlayer: PlayerReading {
    var reads: [CheckedContinuation<PlayerState, Never>] = []
    var images: [CheckedContinuation<Data?, Never>] = []

    func read(_ player: MusicPlayer, pid: pid_t, authorize: Bool) async -> PlayerState {
        #expect(!authorize) // Background reconciliation must not request consent.
        return await withCheckedContinuation { reads.append($0) }
    }

    func artwork(_ player: MusicPlayer, pid: pid_t, track: PlayingTrack) async -> Data? {
        await withCheckedContinuation { images.append($0) }
    }

    func finishRead(_ state: PlayerState) { reads.removeFirst().resume(returning: state) }
    func finishImage(_ data: Data?) { images.removeFirst().resume(returning: data) }
}

@MainActor
private func waitForPlayback(_ predicate: () async -> Bool) async throws {
    let deadline = ContinuousClock.now + .seconds(3)
    while !(await predicate()) {
        try #require(ContinuousClock.now < deadline)
        try await Task.sleep(for: .milliseconds(5))
    }
}

@Test @MainActor
func nowPlayingRejectsLateReadsAndArtworkAfterChangesAndExit() async throws {
    let reader = DelayedPlayer()
    var pid: pid_t? = 10
    let model = NowPlayingModel(readers: [.music: reader], runningProcess: { $0 == .music ? pid : nil })
    defer { model.shutdown() }
    model.setVisible(true)
    try await waitForPlayback { await reader.reads.count == 1 }
    pid = 20 // The old process exits and a new process launches before the read finishes.
    model.refresh()
    await reader.finishRead(.playing(spotifyTrack))
    try await waitForPlayback { await reader.reads.count == 1 }
    #expect(model.playback.selected == nil)
    await reader.finishRead(.playing(musicTrack))
    try await waitForPlayback { await reader.images.count == 1 }
    #expect(model.playback.players[.music]?.track == musicTrack)

    model.refresh(event: .music)
    try await waitForPlayback { await reader.reads.count == 1 }
    await reader.finishRead(.playing(spotifyTrack))
    try await waitForPlayback { await reader.images.count == 2 }
    let image = try #require(NSImage(systemSymbolName: "music.note", accessibilityDescription: nil)?.tiffRepresentation)
    await reader.finishImage(image) // An old cover must not appear on the new track.
    try await Task.sleep(for: .milliseconds(20))
    #expect(model.playback.players[.music]?.track == spotifyTrack)
    #expect(model.artwork == nil)

    pid = nil
    model.refresh()
    await reader.finishImage(image)
    try await Task.sleep(for: .milliseconds(20))
    #expect(model.playback.selected == nil)
    #expect(model.artwork == nil)
}

@MainActor
func renderNowPlayingLab(directory: String) async throws {
    let model = TunicModel(connectAudio: false)
    defer { model.shutdown() }
    for _ in 0..<100 where model.snapshot == nil { try await Task.sleep(for: .milliseconds(10)) }
    model.enqueue(.createProfile(device: "system-output", name: "Desk headphones", source: .flat))
    for _ in 0..<100 where model.snapshot?.selectedProfileId == nil {
        try await Task.sleep(for: .milliseconds(10))
    }
    #expect(model.snapshot?.selectedProfileId != nil)
    let cover = NSImage(size: NSSize(width: 72, height: 72), flipped: false) { bounds in
        NSColor.systemTeal.setFill()
        bounds.fill()
        NSColor.systemIndigo.setFill()
        NSBezierPath(ovalIn: bounds.insetBy(dx: 12, dy: 12)).fill()
        return true
    }
    let cases: [(String, MusicPlayer, PlayerState, NSImage?)] = [
        ("playing", .spotify, .playing(spotifyTrack), cover),
        ("paused", .music, .paused(musicTrack), nil),
        ("empty", .music, .stopped, nil),
    ]
    for (name, player, state, artwork) in cases {
        var playback = PlaybackState()
        playback.receive([player: state])
        let nowPlaying = NowPlayingModel(playback: playback, artwork: artwork)
        let host = NSHostingView(rootView: ContentView(model: model, nowPlaying: nowPlaying)
            .background(Color(nsColor: .windowBackgroundColor)).environment(\.colorScheme, .dark))
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 320, height: 500),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.setContentSize(host.fittingSize)
        window.orderFrontRegardless()
        defer { window.close() }
        try await Task.sleep(for: .milliseconds(250))
        func findMetal(_ view: NSView) -> MTKView? {
            (view as? MTKView) ?? view.subviews.lazy.compactMap(findMetal).first
        }
        let metal = try #require(findMetal(host))
        let renderer = try #require(metal.delegate as? SpectrumRenderer)
        renderer.receiveSpectrum(demoSpectrum, maximumFrequency: 20_000, at: CACurrentMediaTime())
        metal.draw()
        try await Task.sleep(for: .milliseconds(350))
        #expect(host.fittingSize.width == 320)
        let capture = Process()
        capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        capture.arguments = ["-x", "-o", "-l", String(window.windowNumber),
                             directory + "/now-playing-\(name).png", ]
        try capture.run()
        capture.waitUntilExit()
        #expect(capture.terminationStatus == 0)
    }
}
