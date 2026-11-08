// swift-tools-version:5.10
import PackageDescription

// The Rust engine is built first (scripts/build.sh) into ../target/<profile>/libcutout_ffi.a
let rustLibDir = Context.environment["CUTOUT_RUST_LIB_DIR"] ?? "../target/release"

let package = Package(
    name: "CutOut",
    platforms: [.macOS(.v14)],
    targets: [
        .target(
            name: "CCutOut",
            path: "Sources/CCutOut"
        ),
        .executableTarget(
            name: "CutOut",
            dependencies: ["CCutOut"],
            path: "Sources/CutOut",
            swiftSettings: [.unsafeFlags(["-Osize"], .when(configuration: .release))],
            linkerSettings: [
                .unsafeFlags(["-L\(rustLibDir)", "-lcutout_ffi"]),
                .linkedLibrary("c++"),
                .linkedLibrary("iconv"),
                .linkedFramework("CoreML"),
                .linkedFramework("Vision"),
                .linkedFramework("Accelerate"),
            ]
        ),
    ]
)
