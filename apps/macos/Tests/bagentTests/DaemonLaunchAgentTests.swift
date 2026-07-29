import XCTest
@testable import bagent

final class DaemonLaunchAgentTests: XCTestCase {
    func testPlistContainsBinaryLabelAndSortedEnv() throws {
        let plist = DaemonLaunchAgent.plistContent(
            binaryPath: "/Applications/bagent.app/Contents/MacOS/bagentd",
            environment: [
                "BAGENT_BASERT_BASE_URL": "http://127.0.0.1:8082/v1",
                "BAGENT_DEFAULT_MODEL": BaseRTLaunchAgent.model,
            ]
        )
        XCTAssertTrue(plist.contains("<string>com.bagent.daemon</string>"))
        XCTAssertTrue(plist.contains("<string>/Applications/bagent.app/Contents/MacOS/bagentd</string>"))
        // Env is emitted sorted by key so identical settings produce identical plists.
        let defaultRange = plist.range(of: "BAGENT_DEFAULT_MODEL")!
        let baseURLRange = plist.range(of: "BAGENT_BASERT_BASE_URL")!
        XCTAssertLessThan(baseURLRange.lowerBound, defaultRange.lowerBound)
        // Crash-only KeepAlive: clean exits stay down.
        XCTAssertTrue(plist.contains("<key>SuccessfulExit</key>"))
        XCTAssertTrue(plist.contains("<key>RunAtLoad</key>"))
        // Valid XML plist.
        let parsed = try PropertyListSerialization.propertyList(
            from: plist.data(using: .utf8)!, options: [], format: nil) as? [String: Any]
        XCTAssertEqual(parsed?["Label"] as? String, "com.bagent.daemon")
    }

    func testBaseRTPlistKeepsServiceResidentWithLazyRegisteredModels() throws {
        let plist = BaseRTLaunchAgent.plistContent(
            binaryPath: "/Users/oliver/.basert/basert",
            modelDirectory: "/Users/oliver/Library/Application Support/bagent/basert-models"
        )
        XCTAssertTrue(plist.contains("<string>com.bagent.basert</string>"))
        XCTAssertTrue(plist.contains("<string>--model-dir</string>"))
        XCTAssertTrue(plist.contains(
            "<string>/Users/oliver/Library/Application Support/bagent/basert-models</string>"))
        XCTAssertTrue(plist.contains("<string>--idle-timeout</string>"))
        XCTAssertTrue(plist.contains("<string>1200</string>"))
        XCTAssertTrue(plist.contains("<string>--max-context</string>"))
        XCTAssertTrue(plist.contains("<string>4096</string>"))
        XCTAssertTrue(plist.contains("<string>8082</string>"))
        XCTAssertTrue(plist.contains("<string>basert-local</string>"))
        XCTAssertFalse(plist.contains("<string>8080</string>"))
        XCTAssertFalse(plist.contains("<string>basecompute/Qwen3-4B-Instruct-2507</string>"))
        XCTAssertFalse(plist.contains("<string>basecompute/Qwen3.6-35B-A3B</string>"))

        let parsed = try PropertyListSerialization.propertyList(
            from: plist.data(using: .utf8)!, options: [], format: nil
        ) as? [String: Any]
        let args = parsed?["ProgramArguments"] as? [String]
        XCTAssertEqual(args?.first, "/Users/oliver/.basert/basert")
        XCTAssertEqual(args?.dropFirst().first, "serve")
        XCTAssertEqual(args?.filter { $0 == "--model-dir" }.count, 1)
    }
}
