import AppKit
import Combine

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {

    private var notchController: NotchWindowController?
    private var daemonLauncher: DaemonLauncher?
    private var tavilyConfigurationManager: TavilyConfigurationManager?
    private var chatViewModel: ChatViewModel?
    private var browserCoordinator: BrowserCoordinator?
    private var browserCommandServer: BrowserCommandServer?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)

        let launcher = DaemonLauncher()
        launcher.launch()
        daemonLauncher = launcher

        let tavilyConfigurationManager = TavilyConfigurationManager()
        tavilyConfigurationManager.start()
        self.tavilyConfigurationManager = tavilyConfigurationManager

        let vm = ChatViewModel()
        chatViewModel = vm
        let browserCoordinator = BrowserCoordinator()
        browserCoordinator.start()
        self.browserCoordinator = browserCoordinator
        let nc = NotchWindowController(chatViewModel: vm, browserCoordinator: browserCoordinator)
        notchController = nc

        // Background automation approvals preempt everything: open the notch
        // as soon as one arrives (the approval overlay renders before any
        // ordinary mode inside InlineNotchContent).
        vm.onApprovalArrived = { [weak nc] in
            guard let nc, !nc.isNotchInteractionShowing else { return }
            nc.presentInputOnly()
        }
        vm.startEventsMonitor()

        let browserCommandServer = BrowserCommandServer(coordinator: browserCoordinator)
        browserCoordinator.onEnabledChanged = { [weak browserCommandServer] _ in
            // Keep the user-only endpoint alive while the feature is disabled
            // so MCP clients receive browser_disabled instead of an ambiguous
            // startup timeout. The coordinator still creates no session or cue.
            browserCommandServer?.start()
        }
        browserCommandServer.start()
        self.browserCommandServer = browserCommandServer

        GlobalHotkey.register { [weak self] in
            DispatchQueue.main.async { self?.handleHotkey() }
        }
    }

    /// ⌥Space toggles the notch input surface.
    private func handleHotkey() {
        guard let nc = notchController else { return }
        nc.isNotchInteractionShowing ? nc.collapse() : nc.presentInputOnly()
    }

    func applicationWillTerminate(_ notification: Notification) {
        browserCommandServer?.stop()
        if let browserCoordinator {
            for session in browserCoordinator.sessions.values { session.terminate() }
        }
        GlobalHotkey.unregister()
        chatViewModel?.cmuxMonitor.stop()
        tavilyConfigurationManager?.stop()
        daemonLauncher?.stop()
    }
}
