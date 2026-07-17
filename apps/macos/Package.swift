// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "bagent",
    platforms: [.macOS(.v14)],
    dependencies: [],
    targets: [
        .executableTarget(
            name: "bagent",
            dependencies: [],
            path: "Sources/bagent",
            linkerSettings: [
                .linkedFramework("Carbon"),
                .linkedFramework("ScreenCaptureKit"),
                .linkedFramework("Vision"),
                .linkedFramework("ApplicationServices"),
                // Embed Info.plist into the bare executable's __TEXT,__info_plist
                // section so `swift run` (no .app bundle) still carries
                // NSScreenCaptureUsageDescription — required for screen-capture TCC.
                // Path is relative to the package root (linker cwd).
                .unsafeFlags([
                    "-Xlinker", "-sectcreate",
                    "-Xlinker", "__TEXT",
                    "-Xlinker", "__info_plist",
                    "-Xlinker", "Info.plist",
                ]),
            ]
        ),
        .testTarget(
            name: "bagentTests",
            dependencies: ["bagent"],
            path: "Tests/bagentTests"
        ),
    ]
)
