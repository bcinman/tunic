import AppKit
import Carbon

protocol PlayerReading: Sendable {
    func read(_ player: MusicPlayer, pid: pid_t, authorize: Bool) async -> PlayerState
    func artwork(_ player: MusicPlayer, pid: pid_t, track: PlayingTrack) async -> Data?
}

/// Only read Apple Events are sent, addressed to a PID so observing an app can never launch it.
/// Actor isolation keeps synchronous, timeout-bounded Apple Events off the UI thread.
actor PlayerReader: PlayerReading {
    private let session = URLSession(configuration: .ephemeral)

    func read(_ player: MusicPlayer, pid: pid_t, authorize: Bool = false) -> PlayerState {
        let target = NSAppleEventDescriptor(processIdentifier: pid)
        let permission = AEDeterminePermissionToAutomateTarget(target.aeDesc, typeWildCard, typeWildCard, authorize)
        switch permission {
        case OSStatus(errAEEventWouldRequireUserConsent): return .needsAuthorization
        case OSStatus(errAEEventNotPermitted): return .denied
        case noErr: break
        default: return .unavailable
        }
        do {
            let state = try get(property("pPlS"), target: target).enumCodeValue
            guard state == code("kPSP") || state == code("kPSp") else { return .stopped }
            let reference = try get(property("pTrk"), target: target)
            let track = try metadata(reference, player: player, target: target)
            // Current-track references in some players are dynamic. Reject a mixed snapshot.
            let current = try get(property("pTrk"), target: target)
            guard try metadata(current, player: player, target: target) == track,
                  try get(property("pPlS"), target: target).enumCodeValue == state else { return .unavailable }
            return state == code("kPSP") ? .playing(track) : .paused(track)
        } catch {
            return (error as NSError).code == errAEEventNotPermitted ? .denied : .unavailable
        }
    }

    func artwork(_ player: MusicPlayer, pid: pid_t, track: PlayingTrack) async -> Data? {
        let target = NSAppleEventDescriptor(processIdentifier: pid)
        guard AEDeterminePermissionToAutomateTarget(target.aeDesc, typeWildCard, typeWildCard, false) == noErr
        else { return nil }
        do {
            let reference = try get(property("pTrk"), target: target)
            guard try metadata(reference, player: player, target: target) == track else { return nil }
            let data: Data?
            if player == .spotify {
                let value = try get(property("aUrl", of: reference), target: target).stringValue ?? ""
                guard let url = URL(string: value), url.scheme == "https" else { return nil }
                let request = URLRequest(url: url, cachePolicy: .returnCacheDataElseLoad, timeoutInterval: 5)
                let (bytes, response) = try await session.data(for: request)
                guard (response as? HTTPURLResponse)?.statusCode == 200 else { return nil }
                data = bytes
            } else {
                let artwork = object(want: "cArt", form: "indx", selection: NSAppleEventDescriptor(int32: 1),
                                     container: reference)
                data = try get(property("pRaw", of: artwork), target: target).data
            }
            let current = try get(property("pTrk"), target: target)
            guard try metadata(current, player: player, target: target) == track else { return nil }
            return data
        } catch { return nil }
    }

    private func metadata(_ reference: NSAppleEventDescriptor, player: MusicPlayer,
                          target: NSAppleEventDescriptor) throws -> PlayingTrack {
        let title = try get(property("pnam", of: reference), target: target).stringValue ?? ""
        guard !title.isEmpty else { throw NSError(domain: NSOSStatusErrorDomain, code: errAENoSuchObject) }
        let identity = property(player == .music ? "pPIS" : "ID  ", of: reference)
        return try PlayingTrack(
            identifier: get(identity, target: target).stringValue ?? "",
            title: title,
            artist: get(property("pArt", of: reference), target: target).stringValue ?? "",
            album: get(property("pAlb", of: reference), target: target).stringValue ?? "")
    }

    private func get(_ object: NSAppleEventDescriptor,
                     target: NSAppleEventDescriptor) throws -> NSAppleEventDescriptor {
        let event = NSAppleEventDescriptor(eventClass: code("core"), eventID: code("getd"),
                                          targetDescriptor: target, returnID: AEReturnID(kAutoGenerateReturnID),
                                          transactionID: AETransactionID(kAnyTransactionID))
        event.setParam(object, forKeyword: keyDirectObject)
        let reply = try event.sendEvent(options: [.waitForReply, .neverInteract], timeout: 2)
        if let error = reply.paramDescriptor(forKeyword: keyErrorNumber), error.int32Value != 0 {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(error.int32Value))
        }
        guard let result = reply.paramDescriptor(forKeyword: keyDirectObject) else {
            throw NSError(domain: NSOSStatusErrorDomain, code: errAENoSuchObject)
        }
        return result
    }

    private func property(_ name: String, of container: NSAppleEventDescriptor = .null()) -> NSAppleEventDescriptor {
        object(want: "prop", form: "prop", selection: NSAppleEventDescriptor(typeCode: code(name)),
               container: container)
    }

    private func object(want: String, form: String, selection: NSAppleEventDescriptor,
                        container: NSAppleEventDescriptor) -> NSAppleEventDescriptor {
        let record = NSAppleEventDescriptor.record()
        record.setDescriptor(NSAppleEventDescriptor(typeCode: code(want)), forKeyword: AEKeyword(keyAEDesiredClass))
        record.setDescriptor(NSAppleEventDescriptor(enumCode: code(form)), forKeyword: AEKeyword(keyAEKeyForm))
        record.setDescriptor(selection, forKeyword: AEKeyword(keyAEKeyData))
        record.setDescriptor(container, forKeyword: AEKeyword(keyAEContainer))
        return record.coerce(toDescriptorType: typeObjectSpecifier)!
    }

    private func code(_ value: String) -> FourCharCode {
        value.utf8.reduce(0) { ($0 << 8) | UInt32($1) }
    }
}
