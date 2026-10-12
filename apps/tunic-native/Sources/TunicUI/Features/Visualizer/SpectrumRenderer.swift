import MetalKit

@MainActor
final class SpectrumRenderer: NSObject, MTKViewDelegate {
    let device: MTLDevice
    let queue: MTLCommandQueue
    private let fieldPipeline: MTLRenderPipelineState
    var envelope = SpectrumEnvelope()
    var points: [SIMD2<Float>] {
        get { envelope.points }
        set { envelope.points = newValue }
    }
    var style = SpectrumStyle()
    private var presentationGeneration = 0

    func resetPresentation(_ view: MTKView) {
        presentationGeneration += 1
        envelope = SpectrumEnvelope()
        view.layer?.isHidden = true
        updateAnimation(view)
    }

    func receiveSpectrum(_ spectrum: [Float], maximumFrequency: Double, at time: Double) {
        envelope.observe(spectrum, maximumFrequency: maximumFrequency, at: time,
                         attack: style.attack, decay: style.decay, smoothing: style.shapeSmoothing)
    }

    /// Smooth displayed dB height, with milliseconds as exponential time constants.
    func advanceSpectrum(to time: Double) {
        envelope.advance(to: time, attack: style.attack, decay: style.decay)
    }

    func updateAnimation(_ view: MTKView) {
        view.isPaused = !envelope.isAnimating
        view.enableSetNeedsDisplay = view.isPaused
    }

    init(device: MTLDevice) throws {
        self.device = device
        guard let queue = device.makeCommandQueue() else {
            throw NSError(domain: "SpectrumRenderer", code: 1)
        }
        self.queue = queue
        let source = try String(contentsOf: Bundle.module.url(forResource: "Spectrum", withExtension: "metal")!,
                                encoding: .utf8)
        let library = try device.makeLibrary(source: source, options: nil)
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
        descriptor.vertexFunction = library.makeFunction(name: "fieldVertex")
        descriptor.fragmentFunction = library.makeFunction(name: "fieldFragment")
        let attachment = descriptor.colorAttachments[0]!
        attachment.isBlendingEnabled = true
        attachment.sourceRGBBlendFactor = .one
        attachment.sourceAlphaBlendFactor = .one
        attachment.destinationRGBBlendFactor = .oneMinusSourceAlpha
        attachment.destinationAlphaBlendFactor = .oneMinusSourceAlpha
        fieldPipeline = try device.makeRenderPipelineState(descriptor: descriptor)
        super.init()
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}

    func draw(in view: MTKView) {
        guard view.window?.isVisible == true else {
            resetPresentation(view)
            return
        }
        let time = CACurrentMediaTime()
        advanceSpectrum(to: time)
        updateAnimation(view)
        guard let pass = view.currentRenderPassDescriptor,
              let drawable = view.currentDrawable,
              let command = queue.makeCommandBuffer() else { return }
        let scale = Float(view.drawableSize.width / max(view.bounds.width, 1))
        encode(pass: pass, command: command, scale: scale)
        command.present(drawable)
        let generation = presentationGeneration
        let hasPoints = !points.isEmpty
        command.addCompletedHandler { [weak self, weak view] command in
            guard command.status == .completed else { return }
            Task { @MainActor in
                guard let self, let view, view.window?.isVisible == true,
                      self.presentationGeneration == generation else { return }
                if hasPoints { view.layer?.isHidden = false }
            }
        }
        command.commit()
    }

    func encode(pass: MTLRenderPassDescriptor, command: MTLCommandBuffer, scale: Float) {
        guard let encoder = command.makeRenderCommandEncoder(descriptor: pass) else { return }
        if points.count > 1 {
            // The engine supplies 256 bins (2 KiB), within Metal's 4 KiB inline limit.
            encoder.setRenderPipelineState(fieldPipeline)
            points.withUnsafeBytes { bytes in
                encoder.setFragmentBytes(bytes.baseAddress!, length: bytes.count, index: 0)
            }
            let texture = pass.colorAttachments[0].texture!
            var parameters = SpectrumUniforms(
                size: SIMD2(Float(texture.width) / scale, Float(texture.height) / scale),
                scale: scale, count: points.count, style: style)
            encoder.setFragmentBytes(&parameters, length: MemoryLayout<SpectrumUniforms>.stride, index: 1)
            encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
        }
        encoder.endEncoding()
    }
}
