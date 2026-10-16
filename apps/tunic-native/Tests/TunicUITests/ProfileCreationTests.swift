import AppKit
import SwiftUI
import Testing
import TunicEngine
@testable import TunicUI

@Test
func presetSearchAndSuggestedNamesIncludeVariants() {
    let preset = Preset(id: "test", brand: "Sénnheiser", model: "HD650", variant: "2020 pads")
    #expect(ProfileCreationDraft.matches(preset, query: "SENNHEISER hd 650 pads"))
    #expect(!ProfileCreationDraft.matches(preset, query: "HD600"))
    #expect(ProfileCreationDraft.suggestedName(preset) == "Sénnheiser HD650 2020 pads")
}

@Test @MainActor
func creationReceiptWaitsAndRetainsFailedForm() async throws {
    let engine = try Engine(connectAudio: false, databasePath: nil)
    defer { engine.shutdown() }
    for _ in 0..<100 where engine.snapshot() == nil { try await Task.sleep(for: .milliseconds(10)) }
    let initial = try #require(engine.snapshot())
    let receipt = try engine.enqueue(command: .createProfile(device: initial.deviceId, name: "Flat", source: .flat))
    var draft = ProfileCreationDraft(source: .flat, name: "Flat", receipt: receipt)
    let pending = draft.reconcile(initial)
    #expect(!pending)
    #expect(draft.receipt == receipt)
    for _ in 0..<100 where (engine.snapshot()?.processedCommand ?? 0) < receipt {
        try await Task.sleep(for: .milliseconds(10))
    }
    let created = try #require(engine.snapshot())
    let completed = draft.reconcile(created)
    #expect(completed)
    #expect(created.controls.count == 5)
    #expect(created.controls.allSatisfy { $0.gain == 0 })
    let failed = try engine.enqueue(command: .createProfile(
        device: initial.deviceId, name: "Missing", source: .preset(id: "missing")))
    draft = ProfileCreationDraft(source: .preset(id: "missing"), name: "Keep this name",
                                 receipt: failed, previousSelection: created.selectedProfileId)
    for _ in 0..<100 where (engine.snapshot()?.processedCommand ?? 0) < failed {
        try await Task.sleep(for: .milliseconds(10))
    }
    let rejected = draft.reconcile(try #require(engine.snapshot()))
    #expect(!rejected)
    #expect(draft.receipt == nil)
    #expect(draft.name == "Keep this name")
}

@MainActor
private func renderFlatEditor(directory: String) async throws {
    _ = NSApplication.shared
    let model = TunicModel(connectAudio: false)
    defer { model.shutdown() }
    for _ in 0..<100 where model.snapshot == nil { try await Task.sleep(for: .milliseconds(10)) }
    model.enqueue(.createProfile(device: "system-output", name: "Desk headphones", source: .flat))
    for _ in 0..<100 where model.snapshot?.selectedProfileId == nil {
        try await Task.sleep(for: .milliseconds(10))
    }
    #expect(model.snapshot?.controls.map(\.name) == ["Bass", "Warmth", "Mids", "Presence", "Treble"])
    let host = NSHostingView(rootView: ContentView(model: model)
        .background(Color(nsColor: .windowBackgroundColor)))
    let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 320, height: 500),
                          styleMask: [.borderless], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentView = host
    window.setContentSize(host.fittingSize)
    window.orderFrontRegardless()
    defer { window.close() }
    try await Task.sleep(for: .milliseconds(350))
    let capture = Process()
    capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    capture.arguments = ["-x", "-o", "-l", String(window.windowNumber), directory + "/onboarding-Flat-Editor.png"]
    try capture.run()
    capture.waitUntilExit()
    #expect(capture.terminationStatus == 0)
}

/// Real-window capture, shared with renderVisualizerLab. All engine data is in memory.
@MainActor
func renderProfileCreationLab(directory: String) async throws {
    try await renderFlatEditor(directory: directory)
    let engine = try Engine(connectAudio: false, databasePath: nil)
    defer { engine.shutdown() }
    for _ in 0..<100 where engine.snapshot() == nil { try await Task.sleep(for: .milliseconds(10)) }
    let snapshot = try #require(engine.snapshot())
    let preset = try #require(snapshot.presets.first { $0.model == "HD650" })
    let cases: [(String, ProfileCreationDraft)] = [
        ("First-Launch", ProfileCreationDraft()),
        ("Search-Selected", ProfileCreationDraft(query: "HD 650", source: .preset(id: preset.id),
                                                 name: ProfileCreationDraft.suggestedName(preset))),
        ("No-Results", ProfileCreationDraft(query: "no such headphones")),
        ("Flat", ProfileCreationDraft(source: .flat, name: "Desk headphones")),
        ("Waiting-For-Device", ProfileCreationDraft(source: .flat, name: "Desk headphones")),
    ]
    for (name, draft) in cases {
        var state = snapshot
        state.canCreateProfile = name != "Waiting-For-Device"
        var submissions = 0
        let host = NSHostingView(rootView: ProfileCreationView(state: state, draft: .constant(draft),
                                                               cancel: name == "Flat" ? {} : nil,
                                                               submit: { submissions += 1 })
            .frame(width: 320).background(Color(nsColor: .windowBackgroundColor))
            .environment(\.colorScheme, .dark))
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 320, height: 360),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.setContentSize(host.fittingSize)
        window.orderFrontRegardless()
        defer { window.close() }
        try await Task.sleep(for: .milliseconds(350))
        if name == "Flat" || name == "Waiting-For-Device" {
            // Same visible Add button in both states; only availability differs.
            for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                let event = try #require(NSEvent.mouseEvent(with: type, location: NSPoint(x: 60, y: 27),
                    modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: window.windowNumber, context: nil, eventNumber: 0,
                    clickCount: 1, pressure: 1))
                window.sendEvent(event)
            }
            try await Task.sleep(for: .milliseconds(100))
            #expect(submissions == (state.canCreateProfile ? 1 : 0))
        }
        let capture = Process()
        capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        capture.arguments = ["-x", "-o", "-l", String(window.windowNumber), directory + "/onboarding-\(name).png"]
        try capture.run()
        capture.waitUntilExit()
        #expect(capture.terminationStatus == 0)
    }
}
