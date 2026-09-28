import TunicFFI

/// Status returned directly from Tunic's real-time processor entry point.
public enum RealtimeProcessStatus: UInt8 {
    case ok = 0
    case invalidHandle = 1
    case invalidBuffer = 2
    case tooManyFrames = 3
}

/// A retained processor token suitable for one native audio callback.
///
/// Construct and destroy this value off the real-time thread. Calls to
/// `process` neither copy nor take ownership of the supplied audio storage.
public struct RealtimeProcessor {
    private let owner: Processor
    private let handle: UInt64

    public init(_ owner: Processor) {
        self.owner = owner
        self.handle = owner.realtimeHandle()
    }

    public func process(
        interleavedStereo: UnsafeMutablePointer<Float>?,
        frameCount: Int
    ) -> RealtimeProcessStatus {
        guard frameCount >= 0 else {
            return .invalidBuffer
        }
        let status = tunic_processor_process_realtime(
            handle,
            interleavedStereo,
            frameCount
        )
        return RealtimeProcessStatus(rawValue: status) ?? .invalidBuffer
    }
}
