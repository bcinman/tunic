import SwiftUI

enum CellShape: UInt32, CaseIterable {
    case circle, square

    var label: String { self == .circle ? "Circle" : "Square" }
}

/// Ephemeral visual settings only; these never change the audio chain.
struct SpectrumStyle: Equatable {
    var dotSize: Double = 3
    var dotSpacing: Double = 4
    var amplitudeResponse: Double = 0.5
    var cellShape: CellShape = .circle
    var sizeResponse: Double = 1
    var opacityResponse: Double = 0
    var solidFill = false
    var solidColor = Color.white
    var attack: Double = 0
    var decay: Double = 100
    var shapeSmoothing: Double = 0.5
    var gradientStart = Color(red: 0.27, green: 0.91, blue: 1)
    var gradientSecond = Color(red: 0.61, green: 1, blue: 0.94)
    var gradientThird = Color(red: 0.82, green: 0.81, blue: 1)
    var gradientEnd = Color(red: 1, green: 0.68, blue: 0.97)
}
