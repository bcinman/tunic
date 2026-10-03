// swift-tools-version: 6.2

import PackageDescription

let package = Package(
    name: "TunicNative",
    platforms: [
        .macOS(.v14),
    ],
    products: [
        .executable(name: "Tunic", targets: ["Tunic"]),
    ],
    targets: [
        .executableTarget(name: "Tunic"),
    ]
)
