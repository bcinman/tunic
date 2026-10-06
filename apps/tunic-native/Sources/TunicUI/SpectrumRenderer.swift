import MetalKit

@MainActor
final class SpectrumRenderer: NSObject, MTKViewDelegate {
    let device: MTLDevice
    let queue: MTLCommandQueue
    private let pointPipeline: MTLRenderPipelineState
    private let fieldPipeline: MTLRenderPipelineState
    private let bloom: SpectrumBloom
    var envelope = SpectrumEnvelope()
    var points: [SIMD2<Float>] {
        get { envelope.points }
        set { envelope.points = newValue }
    }
    var style = SpectrumStyle()
    private(set) var bassPulse = BassPulse()
    private(set) var ripples = SpectrumRipples()
    private var ripplePreview = 0
    func receiveSpectrum(_ spectrum: [Float], maximumFrequency: Double, at time: Double) {
        envelope.observe(spectrum, maximumFrequency: maximumFrequency, at: time,
                         attack: style.attack, decay: style.decay, smoothing: style.shapeSmoothing)
    }

    /// Smooth displayed dB height, with milliseconds as exponential time constants.
    func advanceSpectrum(to time: Double) {
        envelope.advance(to: time, attack: style.attack, decay: style.decay)
    }

    func advanceRipples(to time: Double) {
        if style.rippleActive {
            ripples.advance(to: time, speed: style.rippleSpeed, decay: style.rippleDecay)
        } else {
            ripples = SpectrumRipples()
        }
    }

    func receiveBass(_ spectrum: [Float], at time: Double) {
        advanceRipples(to: time)
        let drive = style.rippleActive || style.chromaticActive ? bassDrive(spectrum, threshold: style.bassThreshold) : 0
        if style.rippleActive {
            ripples.observe(drive, at: time,
                            preview: style.ripplePreview != ripplePreview)
        }
        ripplePreview = style.ripplePreview
        guard style.chromaticActive, !spectrum.isEmpty else {
            bassPulse = BassPulse()
            return
        }
        bassPulse.observe(drive, at: time, decay: style.chromaticDecay)
    }

    func updateAnimation(_ view: MTKView) {
        view.isPaused = !(envelope.isAnimating || (style.chromaticActive && bassPulse.level > 0)
            || (style.rippleActive && !ripples.waves.isEmpty))
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
        descriptor.vertexFunction = library.makeFunction(name: "spectrumVertex")
        descriptor.fragmentFunction = library.makeFunction(name: "spectrumFragment")
        descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
        pointPipeline = try device.makeRenderPipelineState(descriptor: descriptor)
        descriptor.vertexFunction = library.makeFunction(name: "fieldVertex")
        descriptor.fragmentFunction = library.makeFunction(name: "fieldFragment")
        let attachment = descriptor.colorAttachments[0]!
        attachment.isBlendingEnabled = true
        attachment.sourceRGBBlendFactor = .one
        attachment.sourceAlphaBlendFactor = .one
        attachment.destinationRGBBlendFactor = .oneMinusSourceAlpha
        attachment.destinationAlphaBlendFactor = .oneMinusSourceAlpha
        fieldPipeline = try device.makeRenderPipelineState(descriptor: descriptor)
        bloom = try SpectrumBloom(device: device, library: library)
        super.init()
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}

    func draw(in view: MTKView) {
        guard view.window?.isVisible == true else {
            bassPulse = BassPulse()
            ripples = SpectrumRipples()
            envelope.settle()
            updateAnimation(view)
            return
        }
        let time = CACurrentMediaTime()
        advanceSpectrum(to: time)
        advanceRipples(to: time)
        ripples.retire(beyond: view.bounds.width + style.rippleWidth + style.dotSpacing + style.chromaticStrength)
        bassPulse.advance(to: time, decay: style.chromaticDecay)
        updateAnimation(view)
        guard let pass = view.currentRenderPassDescriptor,
              let drawable = view.currentDrawable,
              let command = queue.makeCommandBuffer() else { return }
        let scale = Float(view.drawableSize.width / max(view.bounds.width, 1))
        encode(pass: pass, command: command, scale: scale)
        command.present(drawable)
        command.commit()
    }

    func encode(pass: MTLRenderPassDescriptor, command: MTLCommandBuffer, scale: Float) {
        if style.deepGlowEnabled && style.deepGlowStrength > 0,
           let output = pass.colorAttachments[0].texture,
           bloom.prepare(width: output.width, height: output.height,
                         radiusPixels: Float(style.deepGlowRadius) * scale), let scene = bloom.scene {
            encodeSpectrum(pass: SpectrumBloom.pass(for: scene), command: command, scale: scale)
            bloom.encode(to: pass, command: command, scale: scale,
                         radius: Float(style.deepGlowRadius), strength: Float(style.deepGlowStrength))
        } else {
            bloom.releaseTextures()
            encodeSpectrum(pass: pass, command: command, scale: scale)
        }
    }

    private func encodeSpectrum(pass: MTLRenderPassDescriptor, command: MTLCommandBuffer, scale: Float) {
        guard let encoder = command.makeRenderCommandEncoder(descriptor: pass) else { return }
        if points.count > 1 {
            // The engine supplies 256 bins (2 KiB), within Metal's 4 KiB inline limit.
            if style.mode == .points {
                encoder.setRenderPipelineState(pointPipeline)
                points.withUnsafeBytes { bytes in
                    encoder.setVertexBytes(bytes.baseAddress!, length: bytes.count, index: 0)
                }
                var size = Float(style.width) * scale
                encoder.setVertexBytes(&size, length: MemoryLayout<Float>.size, index: 1)
                encoder.drawPrimitives(type: .point, vertexStart: 0, vertexCount: points.count)
            } else {
                encoder.setRenderPipelineState(fieldPipeline)
                points.withUnsafeBytes { bytes in
                    encoder.setFragmentBytes(bytes.baseAddress!, length: bytes.count, index: 0)
                }
                let texture = pass.colorAttachments[0].texture!
                var parameters = SpectrumUniforms(
                    size: SIMD2(Float(texture.width) / scale, Float(texture.height) / scale),
                    scale: scale, count: points.count, style: style,
                    chromaticShift: style.chromaticActive ? Float(style.chromaticStrength * bassPulse.level) : 0,
                    ripples: ripples)
                encoder.setFragmentBytes(&parameters, length: MemoryLayout<SpectrumUniforms>.stride, index: 1)
                encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
            }
        }
        encoder.endEncoding()
    }
}
