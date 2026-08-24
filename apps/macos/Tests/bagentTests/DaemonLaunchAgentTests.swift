import XCTest
@testable import bagent

final class DaemonLaunchAgentTests: XCTestCase {
    func testRuntimeEnvironmentDropsRemovedAuthoritySwitch() {
        let environment = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_STAGE8_ACCEPTANCE_FIXTURES": "0",
        ])
        XCTAssertNil(environment["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"])
        XCTAssertNil(environment["BAGENT_SYNTHESIS_FALLBACK_MODEL_PATH"])
        XCTAssertNotNil(environment["BAGENT_CHAT_MODEL_PATH"])
    }

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

        let acceptance = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_STAGE8_ACCEPTANCE_FIXTURES": "1",
        ])
        XCTAssertEqual(acceptance["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"], "1")

        var rollbackWithoutRouting = rollback
        rollbackWithoutRouting.removeValue(forKey: "BAGENT_EVIDENCE_ORCHESTRATOR")
        XCTAssertEqual(rollbackWithoutRouting, ordinary)
    }

    func testRollbackOmitsSubordinateResolverMode() {
        let environment = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_EVIDENCE_ORCHESTRATOR": "0",
            "BAGENT_REFERENCE_RESOLVER_MODE": "enforce",
        ])

        XCTAssertEqual(environment["BAGENT_EVIDENCE_ORCHESTRATOR"], "0")
        XCTAssertNil(environment["BAGENT_REFERENCE_RESOLVER_MODE"])
    }

    func testEnabledTopLevelForwardsSubordinateResolverModeUnchanged() {
        let environment = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_EVIDENCE_ORCHESTRATOR": "1",
            "BAGENT_REFERENCE_RESOLVER_MODE": " observe ",
        ])

        XCTAssertEqual(environment["BAGENT_EVIDENCE_ORCHESTRATOR"], "1")
        XCTAssertEqual(environment["BAGENT_REFERENCE_RESOLVER_MODE"], " observe ")
    }

    func testAbsentTopLevelStillForwardsSubordinateResolverMode() {
        let environment = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_REFERENCE_RESOLVER_MODE": "persistence",
        ])

        XCTAssertNil(environment["BAGENT_EVIDENCE_ORCHESTRATOR"])
        XCTAssertEqual(environment["BAGENT_REFERENCE_RESOLVER_MODE"], "persistence")
    }

    func testInvalidTopLevelPreservesBothValuesForDaemonHandling() {
        let environment = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_EVIDENCE_ORCHESTRATOR": "unexpected",
            "BAGENT_REFERENCE_RESOLVER_MODE": "enforce",
        ])

        XCTAssertEqual(environment["BAGENT_EVIDENCE_ORCHESTRATOR"], "unexpected")
        XCTAssertEqual(environment["BAGENT_REFERENCE_RESOLVER_MODE"], "enforce")
    }

    func testAbsentSubordinateResolverModeAddsNothing() {
        let environment = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_EVIDENCE_ORCHESTRATOR": "1",
        ])

        XCTAssertNil(environment["BAGENT_REFERENCE_RESOLVER_MODE"])
    }

    func testPlistPreservesSelectedResolverEnvironmentExactly() throws {
        let environment = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_EVIDENCE_ORCHESTRATOR": "1",
            "BAGENT_REFERENCE_RESOLVER_MODE": "Enforce & keep",
        ])
        let plist = DaemonLaunchAgent.plistContent(
            binaryPath: "/Applications/bagent.app/Contents/MacOS/bagentd",
            environment: environment
        )
        let parsed = try PropertyListSerialization.propertyList(
            from: Data(plist.utf8), options: [], format: nil
        ) as? [String: Any]
        let serialized = parsed?["EnvironmentVariables"] as? [String: String]

        XCTAssertEqual(serialized?["BAGENT_EVIDENCE_ORCHESTRATOR"], "1")
        XCTAssertEqual(serialized?["BAGENT_REFERENCE_RESOLVER_MODE"], "Enforce & keep")
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

    func testPlistDoesNotAddRemovedAuthoritySwitch() throws {
        let plist = DaemonLaunchAgent.plistContent(
            binaryPath: "/Applications/bagent.app/Contents/MacOS/bagentd",
            environment: DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
                "BAGENT_STAGE8_ACCEPTANCE_FIXTURES": "0",
            ])
        )
        let parsed = try PropertyListSerialization.propertyList(
            from: Data(plist.utf8), options: [], format: nil
        ) as? [String: Any]
        let environment = parsed?["EnvironmentVariables"] as? [String: String]
        XCTAssertNil(environment?["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"])
    }
}
