// swift-tools-version: 6.2

import PackageDescription

let package = Package(
    name: "TunicNative",
    platforms: [
        .macOS(.v26),
    ],
    products: [
        .executable(name: "Tunic", targets: ["Tunic"]),
        .library(name: "TunicUI", type: .dynamic, targets: ["TunicUI"]),
    ],
    dependencies: [
        .package(name: "TunicEngine", path: "../../target/tunic-ffi/apple"),
        .package(url: "https://github.com/sparkle-project/Sparkle", exact: "2.10.0"),
    ],
    targets: [
        .executableTarget(
            name: "Tunic",
            dependencies: ["TunicUI"]
        ),
        .target(
            name: "TunicUI",
            dependencies: [
                .product(name: "TunicEngine", package: "TunicEngine"),
                .product(name: "Sparkle", package: "Sparkle"),
            ],
            resources: [.copy("Features/Visualizer/Spectrum.metal")],
            linkerSettings: [
                // The Rust static library calls Core Audio; SwiftPM must link it explicitly.
                .linkedFramework("CoreAudio"),
                .unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"]),
            ]
        ),
        .testTarget(name: "TunicUITests", dependencies: ["TunicUI"]),
    ]
)
