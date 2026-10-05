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
    dependencies: [
        .package(name: "TunicEngine", path: "../../target/tunic-ffi/apple"),
    ],
    targets: [
        .executableTarget(
            name: "Tunic",
            dependencies: ["TunicUI"]
        ),
        .target(
            name: "TunicUI",
            dependencies: [.product(name: "TunicEngine", package: "TunicEngine")]
        ),
        .testTarget(name: "TunicUITests", dependencies: ["TunicUI"]),
    ]
)
