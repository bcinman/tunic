import MetalKit
import SwiftUI

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
        context.coordinator?.style = style
        context.coordinator?.receiveSpectrum(spectrum, maximumFrequency: maximumFrequency, at: CACurrentMediaTime())
        context.coordinator?.receiveBass(spectrum, at: CACurrentMediaTime())
        context.coordinator?.updateAnimation(view)
        view.needsDisplay = true
    }

    static func dismantleNSView(_ view: MTKView, coordinator: SpectrumRenderer?) {
        view.isPaused = true
        view.delegate = nil
    }
}

/// Peak magnitude in 20–180 Hz; bins retain their original 20 Hz–20 kHz spacing.
func bassDrive(_ spectrum: [Float], threshold: Double) -> Double {
    guard spectrum.count > 1 else { return 0 }
    let last = Int(floor(log(180.0 / 20) / log(1000.0) * Double(spectrum.count - 1)))
    let peak = Double(spectrum[0...last].max() ?? 0)
    let db = 20 * log10(max(peak, 0.000_001))
    return min(1, max(0, (db - threshold) / 18))
}

/// Positive bass changes excite a pulse; elapsed time, not frame count, controls release.
struct BassPulse {
    private(set) var level: Double = 0
    private var previousDrive: Double = 0
    private var lastTime: Double?

    mutating func advance(to time: Double, decay: Double) {
        if let lastTime {
            level *= exp(-max(0, time - lastTime) / (decay / 1000))
            if level < 0.001 { level = 0 }
        }
        lastTime = time
    }

    mutating func observe(_ drive: Double, at time: Double, decay: Double) {
        advance(to: time, decay: decay)
        level = max(level, max(0, drive - previousDrive))
        previousDrive = drive
    }
}

/// Up to four bass fronts travel from left to right; held notes do not retrigger.
struct SpectrumRipples {
    private(set) var waves: [SIMD2<Double>] = [] // distance in points, remaining energy
    private var previousDrive: Double = 0
    private var lastTime: Double?
    private var lastHit: Double = -.infinity

    mutating func advance(to time: Double, speed: Double, decay: Double) {
        let elapsed = max(0, time - (lastTime ?? time))
        lastTime = time
        for index in waves.indices {
            waves[index].x += elapsed * speed
            waves[index].y *= exp(-elapsed * 1000 / decay)
        }
        waves.removeAll { $0.y < 0.001 }
    }

    mutating func retire(beyond distance: Double) {
        waves.removeAll { $0.x > distance }
    }

    mutating func observe(_ drive: Double, at time: Double, preview: Bool = false) {
        let rise = max(0, drive - previousDrive)
        previousDrive = drive
        guard preview || (rise > 0.08 && time - lastHit >= 0.12) else { return }
        lastHit = time
        waves.append(SIMD2(0, preview ? 1 : rise))
        if waves.count > 4 { waves.removeFirst() }
    }
}

/// Clip-space coordinates: bins span 20 Hz–20 kHz logarithmically; height is −90…0 dBFS.
func spectrumPoints(_ amplitudes: [Float], maximumFrequency: Double) -> [SIMD2<Float>] {
    guard amplitudes.count > 1 else { return [] }
    let frequencyScale = log(1_000.0) / log(maximumFrequency / 20)
    return amplitudes.enumerated().map { index, amplitude in
        let db = 20 * log10(max(Double(amplitude), 0.000_001))
        let height = min(1, max(0, (db + 90) / 90))
        return SIMD2(
            Float(2 * Double(index) / Double(amplitudes.count - 1) * frequencyScale - 1),
            Float(2 * height - 1)
        )
    }
}

@MainActor
final class SpectrumRenderer: NSObject, MTKViewDelegate {
    let device: MTLDevice
    let queue: MTLCommandQueue
    private let pointPipeline: MTLRenderPipelineState
    private let fieldPipeline: MTLRenderPipelineState
    var points: [SIMD2<Float>] = []
    var style = SpectrumStyle()
    private(set) var bassPulse = BassPulse()
    private(set) var ripples = SpectrumRipples()
    private var ripplePreview = 0
    private var targetPoints: [SIMD2<Float>] = []
    private var spectrumTime: Double?

    func receiveSpectrum(_ spectrum: [Float], maximumFrequency: Double, at time: Double) {
        advanceSpectrum(to: time)
        targetPoints = spectrumPoints(spectrum, maximumFrequency: maximumFrequency)
        if points.count != targetPoints.count {
            points = targetPoints.map { SIMD2($0.x, -1) }
        }
        advanceSpectrum(to: time)
    }

    /// Smooth displayed dB height, with milliseconds as exponential time constants.
    func advanceSpectrum(to time: Double) {
        let elapsed = max(0, time - (spectrumTime ?? time))
        spectrumTime = time
        for index in targetPoints.indices {
            let target = targetPoints[index]
            let difference = target.y - points[index].y
            let milliseconds = difference > 0 ? style.attack : style.decay
            let fraction = milliseconds == 0 ? 1 : -expm1(-elapsed * 1000 / milliseconds)
            points[index].x = target.x
            points[index].y += difference * Float(fraction)
            if abs(target.y - points[index].y) < 0.0001 { points[index].y = target.y }
        }
    }

    private var chromaticActive: Bool {
        style.mode == .sdf && style.shading == .halftone && style.chromaticEnabled && style.chromaticStrength > 0
    }

    private var rippleActive: Bool {
        style.mode == .sdf && style.shading == .halftone && style.rippleEnabled && style.rippleStrength > 0
    }

    func advanceRipples(to time: Double) {
        if rippleActive {
            ripples.advance(to: time, speed: style.rippleSpeed, decay: style.rippleDecay)
        } else {
            ripples = SpectrumRipples()
        }
    }

    func receiveBass(_ spectrum: [Float], at time: Double) {
        advanceRipples(to: time)
        if rippleActive {
            ripples.observe(bassDrive(spectrum, threshold: style.bassThreshold), at: time,
                            preview: style.ripplePreview != ripplePreview)
        }
        ripplePreview = style.ripplePreview
        guard chromaticActive, !spectrum.isEmpty else {
            bassPulse = BassPulse()
            return
        }
        bassPulse.observe(bassDrive(spectrum, threshold: style.bassThreshold), at: time, decay: style.chromaticDecay)
    }

    func updateAnimation(_ view: MTKView) {
        let smoothing = !targetPoints.isEmpty && points != targetPoints
        view.isPaused = !(smoothing || (chromaticActive && bassPulse.level > 0)
            || (rippleActive && !ripples.waves.isEmpty))
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
        super.init()
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}

    func draw(in view: MTKView) {
        guard view.window?.isVisible == true else {
            bassPulse = BassPulse()
            ripples = SpectrumRipples()
            if !targetPoints.isEmpty { points = targetPoints }
            spectrumTime = nil
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
                // float4s match FieldParameters in Spectrum.metal without padding ambiguity.
                var parameters = [
                    SIMD4<Float>(Float(texture.width) / scale, Float(texture.height) / scale, scale, Float(points.count)),
                    SIMD4<Float>(Float(style.width), Float(style.spread), Float(style.hue), Float(style.shading.rawValue)),
                    SIMD4<Float>(Float(style.dotSize), Float(style.dotSpacing), Float(style.amplitudeResponse),
                                 Float(style.dotPattern.rawValue)),
                    SIMD4<Float>(Float(style.verticalResponse),
                                 chromaticActive ? Float(style.chromaticStrength * bassPulse.level) : 0,
                                 style.whiteHalftone ? 1 : 0, 0),
                    SIMD4<Float>(rippleActive ? Float(style.rippleStrength) : 0,
                                 Float(style.rippleWidth), 0, 0),
                ]
                for index in 0..<4 {
                    let wave = index < ripples.waves.count ? ripples.waves[index] : .zero
                    parameters.append(SIMD4(Float(wave.x), Float(wave.y), 0, 0))
                }
                parameters.withUnsafeBytes { bytes in
                    encoder.setFragmentBytes(bytes.baseAddress!, length: bytes.count, index: 1)
                }
                encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
            }
        }
        encoder.endEncoding()
    }
}
