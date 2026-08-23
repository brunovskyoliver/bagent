import AppKit
import CryptoKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {

    private var notchController: NotchWindowController?
    private var daemonLauncher: DaemonLauncher?
    private var chatViewModel: ChatViewModel?
    private let launchMode: AppLaunchMode
    private var preserveDaemonOnTermination = false
    private var hotkeyGeneration = 0

    init(launchMode: AppLaunchMode = .ordinary) {
        self.launchMode = launchMode
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let stage7CAcceptance = ProcessInfo.processInfo.environment["BAGENT_STAGE7C_ACCEPTANCE_FIXTURE"] == "1"
        NSApp.setActivationPolicy(stage7CAcceptance ? .regular : .accessory)
        let stage7CAcceptanceOld = launchMode.isStage7CAcceptanceOld

        if launchMode.startsDaemon && !stage7CAcceptanceOld {
            let launcher = DaemonLauncher()
            launcher.launch()
            daemonLauncher = launcher
        }

        let handoff: UIRelaunchHandoff?
        if case .uiOnlyRelaunch(let token) = launchMode {
            writeStage7CAcceptanceMarker("replacement-started")
            let replacementIdentity = Bundle.main.bundleIdentifier ?? "sk.bagent.app"
            handoff = try? KeychainProtectedHandoffStore().consume(
                token: token,
                replacement: replacementIdentity,
                now: Date()
            )
            if handoff == nil {
                writeStage7CAcceptanceMarker("replacement-handoff-rejected")
            }
        } else {
            handoff = nil
        }

        let vm = ChatViewModel(
            startMonitoring: launchMode.startsMonitoring,
            consumerFence: handoff?.replacementConsumerFence
        )
        chatViewModel = vm
        let nc = NotchWindowController(
            chatViewModel: vm,
            initiallyHidden: handoff != nil || (launchMode != .ordinary && !stage7CAcceptanceOld)
        )
        notchController = nc

        if launchMode == .ordinary || stage7CAcceptanceOld {
            vm.onUIOnlyRelaunchLaunched = { [weak self, weak vm, weak nc] handoff in
                guard let self, let vm, let nc else { return }
                self.startOldUITakeover(handoff, viewModel: vm, windowController: nc)
            }
        }

        if let handoff {
            Task { @MainActor [weak self, weak vm, weak nc] in
                guard let self, let vm, let nc else { return }
                await self.completeReplacementTakeover(handoff, viewModel: vm, windowController: nc)
            }
        } else if launchMode == .ordinary || stage7CAcceptanceOld {
            vm.startEventsMonitor()
            registerHotkey()
            if stage7CAcceptanceOld {
                Task { @MainActor [weak self, weak vm] in
                    guard let self, let vm else { return }
                    await self.runStage7CAcceptanceOld(viewModel: vm)
                }
            }
        } else {
            vm.recordUIOnlyRelaunchFailure()
            NSApp.terminate(nil)
        }
    }

    private func registerHotkey() {
        hotkeyGeneration += 1
        let generation = hotkeyGeneration
        GlobalHotkey.register { [weak self] in
            DispatchQueue.main.async {
                guard let self, self.hotkeyGeneration == generation else { return }
                self.handleHotkey()
            }
        }
    }

    private func unregisterHotkey() {
        hotkeyGeneration += 1
        GlobalHotkey.unregister()
    }

    private func completeReplacementTakeover(
        _ handoff: UIRelaunchHandoff,
        viewModel: ChatViewModel,
        windowController: NotchWindowController
    ) async {
        let client = DaemonClient()
        let deadline = Date().addingTimeInterval(UIRelaunchTransferMachine.timeout)
        writeStage7CAcceptanceMarker("replacement-takeover-started")
        do {
            // The first frame remains hidden while these two authoritative
            // projections converge. The handoff is only presentation metadata.
            viewModel.restoreUIOnlyRelaunch(handoff)
            writeStage7CAcceptanceMarker("replacement-restored-handoff")
            await viewModel.restoreCurrentChat()
            guard viewModel.currentChatSnapshot?.identity == handoff.currentChatIdentity else {
                writeStage7CAcceptanceMarker("replacement-current-chat-identity-mismatch")
                throw UIRelaunchTransferError.invalidTransition
            }
            writeStage7CAcceptanceMarker("replacement-restored-current")
            let projection = try await client.fetchAuthoritativeSnapshot()
            try viewModel.applyAuthoritativeSnapshot(projection)
            writeStage7CAcceptanceMarker("replacement-fetched-authority")

            _ = try await client.reserveUIRelaunch(
                transferIdentity: handoff.nonce,
                oldConsumerFence: handoff.sourceConsumerFence,
                replacementConsumerFence: handoff.replacementConsumerFence
            )
            let reservedProjection = try await client.fetchReservedUIRelaunchSnapshot(
                transferIdentity: handoff.nonce,
                replacementConsumerFence: handoff.replacementConsumerFence
            )
            writeStage7CAcceptanceMarker("replacement-fetched-reserved")
            try viewModel.applyAuthoritativeSnapshot(reservedProjection)
            writeStage7CAcceptanceMarker("replacement-applied-reserved")
            _ = try await client.markUIRelaunchReady(
                transferIdentity: handoff.nonce,
                replacementConsumerFence: handoff.replacementConsumerFence
            )
            writeStage7CAcceptanceMarker("replacement-ready")

            var oldFenced = false
            var readyObservedAt: Date?
            while Date() < deadline {
                let status = try await client.uiRelaunchStatus(transferIdentity: handoff.nonce).status
                if status == "ready" {
                    readyObservedAt = readyObservedAt ?? Date()
                } else if status == "old_fenced" {
                    oldFenced = true
                } else if status == "rolled_back" || status == "expired" || status == "unknown" {
                    return terminateHiddenReplacement(windowController)
                }

                if !oldFenced,
                   let readyObservedAt,
                   Date().timeIntervalSince(readyObservedAt) >= 1,
                   !sourceUIIsRunning(handoff)
                {
                    // A crashed old UI has already relinquished its local
                    // presentation. The replacement may complete the daemon
                    // fence after proving that the source process is gone.
                    _ = try await client.fenceOldUI(
                        transferIdentity: handoff.nonce,
                        oldConsumerFence: handoff.sourceConsumerFence
                    )
                    oldFenced = true
                }

                guard oldFenced else {
                    try await Task.sleep(for: .milliseconds(100))
                    continue
                }

                _ = try await client.activateUIRelaunch(
                    transferIdentity: handoff.nonce,
                    replacementConsumerFence: handoff.replacementConsumerFence
                )
                let activeProjection = try await client.fetchSnapshot(
                    consumerFence: handoff.replacementConsumerFence
                )
                try viewModel.applyAuthoritativeSnapshot(activeProjection)
                guard await windowController.activateForTakeover() else {
                    writeStage7CAcceptanceActivationEvidence(windowController)
                    writeStage7CAcceptanceMarker("replacement-activation-failed")
                    throw UIRelaunchTransferError.invalidTransition
                }
                writeStage7CAcceptanceMarker("replacement-activated")
                viewModel.startEventsMonitor()

                // This is the first probe from the replacement's signed
                // identity. A process start or successful handoff is not proof.
                guard let permissionKind = permissionKind(for: handoff),
                      let permissionSnapshot = await viewModel.permissions.refreshAuthoritatively(),
                      permissionSnapshot.permissionState(for: permissionKind) == .granted,
                      !permissionSnapshot.requiresUIRelaunch(for: permissionKind) else {
                    writeStage7CAcceptanceMarker("replacement-probe-failed")
                    throw DaemonClient.UIRelaunchTransferError.unavailable
                }
                viewModel.permissions.applyReplacementProbe(permissionSnapshot, for: permissionKind)
                _ = try await client.acknowledgeUIRelaunch(
                    transferIdentity: handoff.nonce,
                    replacementConsumerFence: handoff.replacementConsumerFence
                )
                writeStage7CAcceptanceMarker("replacement-acknowledged")
                writeStage7CAcceptanceEvidence(role: "replacement", viewModel: viewModel, windowController: windowController)
                registerHotkey()
                return
            }
            _ = try? await client.rollbackUIRelaunch(
                transferIdentity: handoff.nonce,
                oldConsumerFence: handoff.sourceConsumerFence
            )
        } catch {
            writeStage7CAcceptanceMarker("replacement-takeover-failed")
            _ = try? await client.rollbackUIRelaunch(
                transferIdentity: handoff.nonce,
                oldConsumerFence: handoff.sourceConsumerFence
            )
        }
        terminateHiddenReplacement(windowController)
    }

    private func sourceUIIsRunning(_ handoff: UIRelaunchHandoff) -> Bool {
        guard let pidText = handoff.sourceUIIdentity.split(separator: ":").last,
              let pid = Int32(pidText),
              let application = NSRunningApplication(processIdentifier: pid_t(pid)) else {
            return false
        }
        return !application.isTerminated
    }

    private func startOldUITakeover(
        _ handoff: UIRelaunchHandoff,
        viewModel: ChatViewModel,
        windowController: NotchWindowController
    ) {
        Task { @MainActor [weak self, weak viewModel, weak windowController] in
            guard let self, let viewModel, let windowController else { return }
            let client = DaemonClient()
            // Keep the old consumer watching until the signed handoff itself
            // expires. A daemon outage must not leave a delayed replacement
            // able to fence an old UI whose watcher already returned.
            let deadline = handoff.expiresAt
            var oldFenced = false
            var oldPaused = false
            while Date() < deadline {
                do {
                    let status = try await client.uiRelaunchStatus(transferIdentity: handoff.nonce).status
                    if status == "ready" && !oldFenced {
                        unregisterHotkey()
                        viewModel.pauseForUIOnlyTakeover()
                        windowController.deactivateForTakeover()
                        oldPaused = true
                        writeStage7CAcceptanceEvidence(
                            role: "old-fenced",
                            viewModel: viewModel,
                            windowController: windowController
                        )
                        do {
                            _ = try await viewModel.fenceUIOnlyTakeover(
                                transferIdentity: handoff.nonce,
                                oldConsumerFence: handoff.sourceConsumerFence
                            )
                            oldFenced = true
                        } catch {
                            // The replacement may have won the same fence
                            // race. Never resume on a failed request until the
                            // daemon proves whether the successor is active.
                            let observed = try? await client.uiRelaunchStatus(
                                transferIdentity: handoff.nonce
                            ).status
                            if let observed,
                               ["old_fenced", "active", "acknowledged"].contains(observed) {
                                oldFenced = true
                            }
                        }
                    } else if status == "acknowledged" {
                        self.preserveDaemonOnTermination = true
                        NSApp.terminate(nil)
                        return
                    } else if status == "rolled_back" || status == "expired" {
                        break
                    }
                } catch {
                    // A daemon outage before activation leaves the old UI as
                    // the local authority. The deadline still bounds waiting.
                }
                try? await Task.sleep(for: .milliseconds(100))
            }

            guard oldPaused else { return }
            let recoveryDeadline = Date().addingTimeInterval(UIRelaunchTransferMachine.timeout)
            var rollbackRestored = false
            while Date() < recoveryDeadline {
                if oldFenced {
                    _ = try? await client.rollbackUIRelaunch(
                        transferIdentity: handoff.nonce,
                        oldConsumerFence: handoff.sourceConsumerFence
                    )
                }
                rollbackRestored = (try? await client.fetchSnapshot(
                    consumerFence: handoff.sourceConsumerFence
                )) != nil
                if rollbackRestored { break }
                try? await Task.sleep(for: .milliseconds(100))
            }
            guard rollbackRestored else { return }
            viewModel.resumeAfterUIOnlyTakeoverRollback()
            guard await windowController.activateForTakeover() else { return }
            writeStage7CAcceptanceEvidence(
                role: "old-restored",
                viewModel: viewModel,
                windowController: windowController
            )
            registerHotkey()
        }
    }

    private func permissionKind(for handoff: UIRelaunchHandoff) -> PermissionGrantKind? {
        guard let child = handoff.selectedChild else { return nil }
        switch child {
        case .fullDiskAccess: return .fullDiskAccess
        case .screenRecording: return .screenRecording
        case .accessibility: return .accessibility
        default: return nil
        }
    }

    private func runStage7CAcceptanceOld(viewModel: ChatViewModel) async {
        writeStage7CAcceptanceMarker("old-started")
        await viewModel.restoreCurrentChat()
        writeStage7CAcceptanceMarker(viewModel.currentChatSnapshot == nil ? "old-no-snapshot" : "old-restored")
        viewModel.openNotchSettings()
        viewModel.openCompassRailChild(.fullDiskAccess)
        writeStage7CAcceptanceMarker("old-before-evidence")
        writeStage7CAcceptanceEvidence(role: "old", viewModel: viewModel, windowController: notchController)
        writeStage7CAcceptanceMarker("old-after-evidence")
        viewModel.requestUIOnlyRelaunch(for: .fullDiskAccess)
    }

    private func writeStage7CAcceptanceEvidence(
        role: String,
        viewModel: ChatViewModel,
        windowController: NotchWindowController?
    ) {
        guard let directory = ProcessInfo.processInfo.environment["BAGENT_STAGE7C_EVIDENCE_DIR"] else {
            writeStage7CAcceptanceMarker("\(role)-evidence-no-directory")
            return
        }
        guard let snapshot = viewModel.currentChatSnapshot else {
            writeStage7CAcceptanceMarker("\(role)-evidence-no-snapshot")
            return
        }
        do {
            let digestInput = "\(snapshot.identity):\(snapshot.revision):\(viewModel.inputText)"
            let digest = SHA256.hash(data: Data(digestInput.utf8)).map { String(format: "%02x", $0) }.joined()
            let evidence: [String: Any] = [
                "role": role,
                "ui_pid": ProcessInfo.processInfo.processIdentifier,
                "current_chat_identity": snapshot.identity,
                "current_chat_revision": snapshot.revision,
                "current_chat_content_sha256": digest,
                "draft_bytes": viewModel.inputText.utf8.count,
                "compass_rail_route": viewModel.compassRailRoute.identifier,
                "consumer_fence": viewModel.activeConsumerFence,
                "presentation_active": windowController?.isTakeoverPresentationActive ?? false,
            ]
            let data = try JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys])
            try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
            try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("\(role).json"), options: .atomic)
        } catch {
            // Acceptance evidence is best-effort; the transfer itself remains
            // authoritative in the daemon and the signed process state.
            writeStage7CAcceptanceMarker("\(role)-evidence-failed")
        }
    }

    private func writeStage7CAcceptanceMarker(_ marker: String) {
        guard let directory = ProcessInfo.processInfo.environment["BAGENT_STAGE7C_EVIDENCE_DIR"] else { return }
        try? FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        try? Data(marker.utf8).write(
            to: URL(fileURLWithPath: directory).appendingPathComponent("\(marker).marker"),
            options: .atomic)
    }

    private func writeStage7CAcceptanceActivationEvidence(_ windowController: NotchWindowController) {
        guard let directory = ProcessInfo.processInfo.environment["BAGENT_STAGE7C_EVIDENCE_DIR"] else { return }
        do {
            let data = try JSONSerialization.data(
                withJSONObject: windowController.takeoverPresentationEvidence,
                options: [.prettyPrinted, .sortedKeys])
            try data.write(
                to: URL(fileURLWithPath: directory).appendingPathComponent("replacement-activation-state.json"),
                options: .atomic)
        } catch {
            writeStage7CAcceptanceMarker("replacement-activation-state-write-failed")
        }
    }

    private func terminateHiddenReplacement(_ windowController: NotchWindowController) {
        windowController.deactivateForTakeover()
        NSApp.terminate(nil)
    }

    /// ⌥Space toggles the notch input surface.
    private func handleHotkey() {
        guard let nc = notchController else { return }
        nc.isNotchInteractionShowing ? nc.collapse() : nc.presentInputOnly()
    }

    func applicationWillTerminate(_ notification: Notification) {
        unregisterHotkey()
        chatViewModel?.cmuxMonitor.stop()
        if !preserveDaemonOnTermination {
            daemonLauncher?.stop()
        }
    }
}
