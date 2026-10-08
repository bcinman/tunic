import Metal

/// Post-chromatic, five-octave bloom. Render targets are private and reused until
/// the drawable or radius changes; disabling releases them in the renderer.
@MainActor
final class SpectrumBloom {
    private let device: MTLDevice
    private let blurPipeline: MTLRenderPipelineState
    private let compositePipeline: MTLRenderPipelineState
    private(set) var scene: MTLTexture?
    private var levels: [(horizontal: MTLTexture, blurred: MTLTexture)] = []
    private var resolutionRadius: Float = 0

    init(device: MTLDevice, library: MTLLibrary) throws {
        self.device = device
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.vertexFunction = library.makeFunction(name: "fieldVertex")
        descriptor.fragmentFunction = library.makeFunction(name: "bloomBlur")
        descriptor.colorAttachments[0].pixelFormat = .rgba16Float
        blurPipeline = try device.makeRenderPipelineState(descriptor: descriptor)
        descriptor.fragmentFunction = library.makeFunction(name: "bloomComposite")
        descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
        compositePipeline = try device.makeRenderPipelineState(descriptor: descriptor)
    }

    func prepare(width: Int, height: Int, radiusPixels: Float) -> Bool {
        if scene?.width == width, scene?.height == height, resolutionRadius == radiusPixels { return true }
        func texture(_ divisor: Int, format: MTLPixelFormat) -> MTLTexture? {
            let descriptor = MTLTextureDescriptor.texture2DDescriptor(
                pixelFormat: format, width: max(1, width / divisor), height: max(1, height / divisor), mipmapped: false)
            descriptor.storageMode = .private
            descriptor.usage = [.renderTarget, .shaderRead]
            return device.makeTexture(descriptor: descriptor)
        }
        releaseTextures()
        for level in 0..<5 {
            // Keep texels no wider than this octave's Gaussian sigma. A fixed
            // mip chain would impose a minimum blur even at small radius settings.
            let divisor = max(1, Int(radiusPixels * Float(1 << level) / 48))
            guard let horizontal = texture(divisor, format: .rgba16Float),
                  let blurred = texture(divisor, format: .rgba16Float) else {
                releaseTextures()
                return false
            }
            levels.append((horizontal, blurred))
        }
        scene = texture(1, format: .bgra8Unorm)
        resolutionRadius = radiusPixels
        return scene != nil
    }

    func releaseTextures() {
        scene = nil
        levels.removeAll(keepingCapacity: true)
    }

    static func pass(for texture: MTLTexture) -> MTLRenderPassDescriptor {
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = texture
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].storeAction = .store
        pass.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 0)
        return pass
    }

    func encode(to pass: MTLRenderPassDescriptor, command: MTLCommandBuffer,
                scale: Float, radius: Float, strength: Float) {
        guard let scene, levels.count == 5 else { return }
        func blur(_ source: MTLTexture, into target: MTLTexture,
                  radius: Float, horizontal: Bool, decode: Bool = false) {
            guard let encoder = command.makeRenderCommandEncoder(descriptor: Self.pass(for: target)) else { return }
            encoder.setRenderPipelineState(blurPipeline)
            encoder.setFragmentTexture(source, index: 0)
            // UV step is based on full-size pixels, independent of downsample level.
            var parameters = SIMD4<Float>(Float(target.width), Float(target.height),
                horizontal ? radius * scale / (6 * Float(scene.width)) : 0,
                horizontal ? 0 : radius * scale / (6 * Float(scene.height)))
            encoder.setFragmentBytes(&parameters, length: MemoryLayout<SIMD4<Float>>.stride, index: 0)
            var decodeFlag: UInt32 = decode ? 1 : 0
            encoder.setFragmentBytes(&decodeFlag, length: MemoryLayout<UInt32>.stride, index: 1)
            encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
            encoder.endEncoding()
        }
        var source = scene
        for (index, level) in levels.enumerated() {
            let levelRadius = radius * Float(1 << index) / 16
            blur(source, into: level.horizontal, radius: levelRadius, horizontal: true, decode: index == 0)
            blur(level.horizontal, into: level.blurred, radius: levelRadius, horizontal: false)
            source = level.blurred
        }
        guard let encoder = command.makeRenderCommandEncoder(descriptor: pass) else { return }
        encoder.setRenderPipelineState(compositePipeline)
        encoder.setFragmentTexture(scene, index: 0)
        for (index, level) in levels.enumerated() {
            encoder.setFragmentTexture(level.blurred, index: index + 1)
        }
        var parameters = SIMD4<Float>(Float(scene.width), Float(scene.height), strength, scale)
        encoder.setFragmentBytes(&parameters, length: MemoryLayout<SIMD4<Float>>.stride, index: 0)
        encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
        encoder.endEncoding()
    }
}
