import SwiftUI

enum SpectrumMode: String, CaseIterable {
    case points = "Points"
    case sdf = "SDF"
}

enum SpectrumShading: UInt32, CaseIterable {
    case line, glow, bands, distance, halftone

    var label: String {
        switch self {
        case .line: "Line"
        case .glow: "Glow"
        case .bands: "Bands"
        case .distance: "Distance"
        case .halftone: "Halftone"
        }
    }
}

enum HalftonePattern: UInt32, CaseIterable {
    case grid, hex

    var label: String { self == .grid ? "Grid" : "Hex" }
}

/// Ephemeral visual settings only; these never change the audio chain.
struct SpectrumStyle: Equatable {
    var mode: SpectrumMode = .sdf
    var shading: SpectrumShading = .halftone
    var width: Double = 2
    var spread: Double = 10
    var hue: Double = 0.52
    var dotSize: Double = 3
    var dotSpacing: Double = 4
    var amplitudeResponse: Double = 0.5
    var dotPattern: HalftonePattern = .grid
    var chromaticEnabled = true
    var chromaticStrength: Double = 8
    var bassThreshold: Double = -42
    var chromaticDecay: Double = 250
    var whiteHalftone = false
    var invertedHalftone = false
    var attack: Double = 0
    var decay: Double = 100
    var shapeSmoothing: Double = 0.5
    var deepGlowEnabled = false
    var deepGlowStrength: Double = 2
    var deepGlowRadius: Double = 32
    var gradientStart = Color(red: 0.27, green: 0.91, blue: 1)
    var gradientSecond = Color(red: 0.61, green: 1, blue: 0.94)
    var gradientThird = Color(red: 0.82, green: 0.81, blue: 1)
    var gradientEnd = Color(red: 1, green: 0.68, blue: 0.97)

    var usesHalftone: Bool { mode == .sdf && shading == .halftone }
    var chromaticActive: Bool { usesHalftone && chromaticEnabled && chromaticStrength > 0 }
}
