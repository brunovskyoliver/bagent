// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "bagent",
    platforms: [.macOS(.v14)],
    dependencies: [
        // Local on-device Whisper STT (CoreML/ANE). Product "WhisperKit" lives in
        // the argmax-oss-swift package.
        .package(url: "https://github.com/argmaxinc/WhisperKit.git", from: "0.9.0"),
    ],
    targets: [
        .executableTarget(
            name: "bagent",
            dependencies: [
                .product(name: "WhisperKit", package: "WhisperKit"),
            ],
            path: "Sources/bagent",
            linkerSettings: [
                .linkedFramework("Carbon"),
                .linkedFramework("WebKit"),
                .linkedFramework("AVFoundation"),
                .linkedFramework("ScreenCaptureKit"),
                .linkedFramework("Vision"),
                .linkedFramework("ApplicationServices"),
                // Embed Info.plist into the bare executable's __TEXT,__info_plist
                // section so `swift run` (no .app bundle) still carries
                // NSMicrophoneUsageDescription — required for microphone/TCC access.
                // Path is relative to the package root (linker cwd).
                .unsafeFlags([
                    "-Xlinker", "-sectcreate",
                    "-Xlinker", "__TEXT",
                    "-Xlinker", "__info_plist",
                    "-Xlinker", "Info.plist",
                ]),
            ]
        ),
    ]
)
