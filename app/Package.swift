// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "Aegis",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Aegis", targets: ["Aegis"])
    ],
    targets: [
        .executableTarget(
            name: "Aegis",
            path: "Aegis",
            exclude: [
                "Resources/Info.plist",
                "Resources/AppIcon.icns",
                "Resources/AppIcon.iconset",
                "Resources/aegis-icon-1024.png",
            ]
        )
    ]
)
