import MetalKit
import SwiftUI

/// SwiftUI owns inputs; the coordinator owns GPU resources and animation lifetime.
struct SpectrumView: NSViewRepresentable {
    let spectrum: [Float]
    let maximumFrequency: Double
    var style = SpectrumStyle()

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
        let view = MTKView(frame: .zero, device: context.coordinator?.device)
        view.colorPixelFormat = .bgra8Unorm
        view.clearColor = MTLClearColorMake(0, 0, 0, 0)
        view.layer?.isOpaque = false
        view.isPaused = true
        view.enableSetNeedsDisplay = true
        view.delegate = context.coordinator
        return view
    }

    func updateNSView(_ view: MTKView, context: Context) {
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
