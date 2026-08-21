import XCTest
@testable import bagent

final class ApplicationDragTests: XCTestCase {
    func testValidSignedApplicationProducesExactPublicFileURLPayload() throws {
        let app = try makeApp(identifier: "sk.bagent.app")
        let result = ApplicationDragValidator.validate(
            appURL: app,
            expectedIdentifier: "sk.bagent.app",
            expectedTeamID: "QUB47S3XTF",
            signature: .valid(identifier: "sk.bagent.app", teamID: "QUB47S3XTF")
        )

        XCTAssertEqual(result, .valid)
        XCTAssertEqual(ApplicationDragRepresentation.contentTypes, ["public.file-url"])
        XCTAssertEqual(ApplicationDragRepresentation.fileURL(for: app), app)
    }

    func testWrongIdentityTeamAdHocAndMissingBundleFailClosed() throws {
        let app = try makeApp(identifier: "sk.bagent.app")
        XCTAssertEqual(validate(app, signature: .valid(identifier: "wrong", teamID: "QUB47S3XTF")), .wrongIdentifier)
        XCTAssertEqual(validate(app, signature: .valid(identifier: "sk.bagent.app", teamID: "OTHER")), .wrongTeam)
        XCTAssertEqual(validate(app, signature: .adHoc), .adHocSignature)
        XCTAssertEqual(
            ApplicationDragValidator.validate(
                appURL: app.appendingPathComponent("missing.app"),
                expectedIdentifier: "sk.bagent.app",
                expectedTeamID: "QUB47S3XTF",
                signature: .invalid
            ),
            .missingBundle
        )
    }

    func testExecutableImageAliasAndFilePromiseFailClosed() throws {
        let app = try makeApp(identifier: "sk.bagent.app")
        let executable = app.appendingPathComponent("Contents/MacOS/bagent")
        FileManager.default.createFile(atPath: executable.path, contents: Data())
        let image = app.deletingLastPathComponent().appendingPathComponent("bagent.png")
        FileManager.default.createFile(atPath: image.path, contents: Data())

        XCTAssertEqual(ApplicationDragValidator.rejection(for: executable), .notApplicationBundle)
        XCTAssertEqual(ApplicationDragValidator.rejection(for: image), .notApplicationBundle)
        XCTAssertEqual(ApplicationDragValidator.rejection(for: app, isAlias: true), .alias)
        XCTAssertEqual(ApplicationDragValidator.rejection(for: app, isFilePromise: true), .filePromise)
    }

    func testAccessibilityMetadataIsStable() {
        XCTAssertEqual(ApplicationDragRepresentation.accessibilityLabel, "Draggable bagent application")
        XCTAssertTrue(ApplicationDragRepresentation.accessibilityHint.contains("signed running application"))
        XCTAssertTrue(ApplicationDragRepresentation.nativeAddFlowHint.contains("System Settings"))
    }

    private func validate(_ app: URL, signature: ApplicationSignature) -> ApplicationDragValidationResult {
        ApplicationDragValidator.validate(
            appURL: app,
            expectedIdentifier: "sk.bagent.app",
            expectedTeamID: "QUB47S3XTF",
            signature: signature
        )
    }

    private func makeApp(identifier: String) throws -> URL {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let app = root.appendingPathComponent("bagent.app")
        try FileManager.default.createDirectory(
            at: app.appendingPathComponent("Contents/MacOS"), withIntermediateDirectories: true
        )
        let plist = "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>\(identifier)</string></dict></plist>"
        try plist.write(to: app.appendingPathComponent("Contents/Info.plist"), atomically: true, encoding: .utf8)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return app
    }
}
