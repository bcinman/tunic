import AppKit
import SwiftUI
import TunicEngine

struct GainControlRow: View {
    private enum Interaction {
        case idle
        case dragging(Double)
        case awaitingFinish(value: Double, receipt: UInt64)
        case cancelled
    }

    let profile: String
    let control: Control
    let processedCommand: UInt64
    let send: (EngineCommand) -> UInt64?
    var dragChanged: (Bool) -> Void = { _ in }
    @State private var interaction = Interaction.idle
    @State private var hovered = false
    @State private var editingName = false
    @State private var name = ""
    @State private var nameSelection: TextSelection?
    @State private var pendingName: (value: String, receipt: UInt64)?
    @State private var outsideClickMonitor: Any?
    @FocusState private var nameFocused: Bool

    private var displayedName: String {
        if let pendingName, processedCommand < pendingName.receipt { return pendingName.value }
        return control.name
    }

    private var gain: Double {
        switch interaction {
        case .dragging(let value), .awaitingFinish(let value, _): value
        case .idle, .cancelled: control.gain
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                if editingName {
                    TextField("Filter name", text: $name, selection: $nameSelection)
                        .textFieldStyle(.plain)
                        .focused($nameFocused)
                        .onSubmit(saveName)
                        .onExitCommand(perform: endNameEditing)
                        .onAppear {
                            nameFocused = true
                            nameSelection = TextSelection(range: name.startIndex..<name.endIndex)
                            outsideClickMonitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { event in
                                if nameFocused, let editor = event.window?.firstResponder as? NSTextView,
                                   !editor.bounds.contains(editor.convert(event.locationInWindow, from: nil)) {
                                    saveName()
                                }
                                return event
                            }
                        }
                        .onDisappear {
                            if let outsideClickMonitor { NSEvent.removeMonitor(outsideClickMonitor) }
                            outsideClickMonitor = nil
                        }
                        .onChange(of: nameFocused) { if !nameFocused { saveName() } }
                } else {
                    Text(displayedName).lineLimit(1)
                        .onTapGesture(count: 2) {
                            name = displayedName
                            editingName = true
                        }
                        .help("Double-click to rename")
                }
                if gain != 0 {
                    Button { changeGain(0) } label: {
                        Image(systemName: "arrow.counterclockwise")
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .opacity(hovered ? 1 : 0)
                    .animation(.easeInOut(duration: 0.1), value: hovered)
                    .accessibilityLabel("Reset \(displayedName) to 0 decibels")
                    .help("Reset to 0 dB")
                }
                Spacer()
                Text("\(gain > 0 ? "+" : "")\(gain.formatted(.number.precision(.fractionLength(0...1)))) dB")
                    .font(.system(size: 11))
                    .monospacedDigit()
                    .foregroundStyle(.opacity(0.5))
            }
            .font(.system(size: 12))
            Slider(value: Binding(get: { gain }, set: { changeGain($0) }),
                   in: -12...12, neutralValue: 0, label: { Text(displayedName) },
                   onEditingChanged: editingChanged)
            .labelsHidden()
            .accessibilityValue("\(gain.formatted()) decibels")
        }.padding(.horizontal, 16)
        .contentShape(Rectangle())
        .onHover { hovered = $0 }
        .onChange(of: processedCommand) {
            guard case .awaitingFinish(_, let receipt) = interaction,
                  processedCommand >= receipt else { return }
            interaction = .idle
        }
        .onExitCommand {
            guard case .dragging = interaction else { return }
            interaction = .cancelled
            dragChanged(false)
            _ = send(.cancelControlGain(profile: profile))
        }
        .onDisappear { editingChanged(false) }
    }

    private func saveName() {
        guard editingName else { return }
        if let receipt = send(.renameControl(profile: profile, filter: control.filter, name: name)) {
            pendingName = (name.trimmingCharacters(in: .whitespacesAndNewlines), receipt)
        }
        endNameEditing()
    }

    private func endNameEditing() {
        let ownedFocus = nameFocused
        editingName = false
        nameFocused = false
        // Resign the field editor before removing it so AppKit need not choose
        // another control. Leave intentional focus moves (e.g. Tab) alone.
        if ownedFocus, let window = NSApp.keyWindow, window.firstResponder is NSTextView {
            window.makeFirstResponder(nil)
        }
    }

    private func changeGain(_ value: Double) {
        if case .cancelled = interaction { return }
        let accepted = send(.previewControlGain(profile: profile, filter: control.filter, gain: value)) != nil
        if case .dragging = interaction {
            interaction = .dragging(accepted ? value : control.gain)
        } else if accepted {
            // Keyboard and accessibility changes may arrive without a drag lifecycle.
            finish(value)
        } else {
            interaction = .idle
        }
    }

    private func editingChanged(_ editing: Bool) {
        if editing {
            interaction = .dragging(gain)
            dragChanged(true)
        } else if case .dragging(let value) = interaction {
            finish(value)
            dragChanged(false)
        } else if case .cancelled = interaction {
            interaction = .idle
        }
    }

    private func finish(_ value: Double) {
        if let receipt = send(.finishControlGain(profile: profile)) {
            interaction = .awaitingFinish(value: value, receipt: receipt)
        } else {
            interaction = .idle
        }
    }
}
