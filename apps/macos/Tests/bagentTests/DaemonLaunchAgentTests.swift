import XCTest
@testable import bagent

final class DaemonLaunchAgentTests: XCTestCase {
    func testRuntimeEnvironmentPreservesExplicitEvidenceRollbackOnly() {
        let rollback = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_EVIDENCE_ORCHESTRATOR": "0",
            "BAGENT_STAGE8_ACCEPTANCE_FIXTURES": "0",
        ])
        XCTAssertEqual(rollback["BAGENT_EVIDENCE_ORCHESTRATOR"], "0")
        XCTAssertNil(rollback["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"])

        let invalid = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_EVIDENCE_ORCHESTRATOR": "unexpected",
        ])
        XCTAssertEqual(invalid["BAGENT_EVIDENCE_ORCHESTRATOR"], "unexpected")

        let ordinary = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [:])
        XCTAssertNil(ordinary["BAGENT_EVIDENCE_ORCHESTRATOR"])
        XCTAssertNil(ordinary["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"])
        XCTAssertNil(ordinary["BAGENT_SYNTHESIS_FALLBACK_MODEL_PATH"])
        XCTAssertNotNil(ordinary["BAGENT_CHAT_MODEL_PATH"])

        var rollbackWithoutRouting = rollback
        rollbackWithoutRouting.removeValue(forKey: "BAGENT_EVIDENCE_ORCHESTRATOR")
        XCTAssertEqual(rollbackWithoutRouting, ordinary)
    }

    func testPlistContainsBinaryLabelAndSortedEnv() throws {
        let plist = DaemonLaunchAgent.plistContent(
            binaryPath: "/Applications/bagent.app/Contents/MacOS/bagentd",
            environment: [
                "BAGENT_BASERT_BASE_URL": "http://127.0.0.1:8082/v1",
                "BAGENT_DEFAULT_MODEL": ModelRuntimeConfiguration.model,
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

    func testPlistEscapesExplicitInvalidEvidenceValueWithoutChangingIt() throws {
        let value = "unexpected<&\"value"
        let plist = DaemonLaunchAgent.plistContent(
            binaryPath: "/Applications/bagent.app/Contents/MacOS/bagentd",
            environment: ["BAGENT_EVIDENCE_ORCHESTRATOR": value]
        )
        let parsed = try PropertyListSerialization.propertyList(
            from: Data(plist.utf8), options: [], format: nil
        ) as? [String: Any]
        let environment = parsed?["EnvironmentVariables"] as? [String: String]
        XCTAssertEqual(environment?["BAGENT_EVIDENCE_ORCHESTRATOR"], value)
    }
}
