import MetalKit
import SwiftUI

/// SwiftUI owns inputs; the coordinator owns GPU resources and animation lifetime.
struct SpectrumView: NSViewRepresentable {
    let spectrum: [Float]
    let maximumFrequency: Double
    var style = SpectrumStyle()
    var hoverChanged: ((Bool) -> Void)?

    func makeCoordinator() -> SpectrumRenderer? {
        guard let device = MTLCreateSystemDefaultDevice() else { return nil }
        do {
            return try SpectrumRenderer(device: device)
        } catch {
            NSLog("Spectrum renderer initialization failed: %@", String(describing: error))
            return nil
        }
    }

    func makeNSView(context: Context) -> MTKView {
        let view = SpectrumMetalView(frame: .zero, device: context.coordinator?.device)
        view.colorPixelFormat = .bgra8Unorm
        view.clearColor = MTLClearColorMake(0, 0, 0, 0)
        view.layer?.isOpaque = false
        view.layer?.isHidden = true
        view.isPaused = true
        view.enableSetNeedsDisplay = true
        view.delegate = context.coordinator
        return view
    }

    func updateNSView(_ view: MTKView, context: Context) {
        (view as? SpectrumMetalView)?.hoverChanged = hoverChanged
        let time = CACurrentMediaTime()
        context.coordinator?.style = style
        context.coordinator?.receiveSpectrum(spectrum, maximumFrequency: maximumFrequency, at: time)
        context.coordinator?.receiveBass(spectrum, at: time)
        context.coordinator?.updateAnimation(view)
        view.needsDisplay = true
    }

    static func dismantleNSView(_ view: MTKView, coordinator: SpectrumRenderer?) {
        view.isPaused = true
        view.delegate = nil
    }
}

/// MenuBarExtra retains its window between presentations, including Metal's last drawable.
final class SpectrumMetalView: MTKView {
    var hoverChanged: ((Bool) -> Void)?
    private var hoverTracking: NSTrackingArea?
    private var visibilityObservation: NSKeyValueObservation?
    private var fpsTimer: Timer?
    private var frameRate = SpectrumFrameRate()
    private let fpsLabel = NSTextField(labelWithString: "0 FPS")

    override init(frame: NSRect, device: MTLDevice?) {
        super.init(frame: frame, device: device)
        fpsLabel.font = .monospacedDigitSystemFont(ofSize: 10, weight: .medium)
        fpsLabel.textColor = .secondaryLabelColor
        fpsLabel.toolTip = "GPU-completed frames per second. Idle visuals stop drawing; this is not a maximum-performance benchmark."
        fpsLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(fpsLabel)
        NSLayoutConstraint.activate([
            fpsLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            fpsLabel.topAnchor.constraint(equalTo: topAnchor, constant: 6),
        ])
    }

    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    isolated deinit { fpsTimer?.invalidate() }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverTracking { removeTrackingArea(hoverTracking) }
        let area = NSTrackingArea(rect: .zero,
                                  options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
                                  owner: self, userInfo: nil)
        addTrackingArea(area)
        hoverTracking = area
    }

    override func mouseEntered(with event: NSEvent) { hoverChanged?(true) }
    override func mouseExited(with event: NSEvent) { hoverChanged?(false) }

    func recordCompletedFrame() {
        frameRate.recordFrame()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        visibilityObservation = window?.observe(\.isVisible, options: [.new]) { [weak self] _, _ in
            MainActor.assumeIsolated { self?.visibilityChanged() }
        }
        visibilityChanged()
    }

    private func visibilityChanged() {
        if window?.isVisible != true {
            fpsTimer?.invalidate()
            fpsTimer = nil
            fpsLabel.stringValue = "0 FPS"
            (delegate as? SpectrumRenderer)?.resetPresentation(self)
        } else if fpsTimer == nil {
            frameRate = SpectrumFrameRate(start: CACurrentMediaTime())
            let timer = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    let fps = self.frameRate.sample(at: CACurrentMediaTime())
                    self.fpsLabel.stringValue = String(format: "%.0f FPS", fps)
                }
            }
            fpsTimer = timer
            RunLoop.main.add(timer, forMode: .common)
        }
    }
}

/// Count completed frames over elapsed wall time, including idle time between draws.
struct SpectrumFrameRate {
    var start: Double = 0
    private var frames = 0

    mutating func recordFrame() { frames += 1 }

    mutating func sample(at time: Double) -> Double {
        let elapsed = time - start
        guard elapsed > 0 else { return 0 }
        let rate = Double(frames) / elapsed
        frames = 0
        start = time
        return rate
    }
}
