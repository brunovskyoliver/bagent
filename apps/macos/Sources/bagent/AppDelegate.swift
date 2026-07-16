import AppKit
import Combine

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {

    private var notchController: NotchWindowController?
    private var daemonLauncher: DaemonLauncher?
    private var chatViewModel: ChatViewModel?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)

        let launcher = DaemonLauncher()
        launcher.launch()
        daemonLauncher = launcher

        let vm = ChatViewModel()
        chatViewModel = vm
        notchController = NotchWindowController(chatViewModel: vm)

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
        GlobalHotkey.unregister()
        chatViewModel?.cmuxMonitor.stop()
        daemonLauncher?.stop()
    }
}
