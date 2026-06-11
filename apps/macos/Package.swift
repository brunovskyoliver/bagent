// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "bagent",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "bagent",
            path: "Sources/bagent",
            linkerSettings: [
                .linkedFramework("Carbon"),
                .linkedFramework("WebKit"),
            ]
        ),
    ]
)
