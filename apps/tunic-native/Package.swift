// swift-tools-version: 6.2

import PackageDescription

let package = Package(
    name: "TunicNative",
    platforms: [
        .macOS(.v26),
    ],
    products: [
        .executable(name: "Tunic", targets: ["Tunic"]),
    ],
    targets: [
        .executableTarget(
            name: "Tunic",
            dependencies: ["TunicUI"]
        ),
        .target(name: "TunicUI"),
    ]
)
