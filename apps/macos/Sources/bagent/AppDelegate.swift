import AppKit
import Combine

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {

    private var notchController: NotchWindowController?
    private var statusBar: StatusBarController?
    private var daemonLauncher: DaemonLauncher?
    private var approvalObserver: AnyCancellable?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)

        let launcher = DaemonLauncher()
        launcher.launch()
        daemonLauncher = launcher

        let vm = ChatViewModel()
        notchController = NotchWindowController(chatViewModel: vm)

        // On notch Mac the pill below the notch is the primary indicator;
        // on external display keep the status item as a right-side fallback.
        if !vm.hasNotch {
            let sb = StatusBarController { [weak self] in
                self?.notchController?.toggle()
            }
            statusBar = sb
            approvalObserver = vm.$pendingApprovals.sink { [weak sb] items in
                sb?.setBadge(items.count)
            }
        }

        GlobalHotkey.register { [weak self] in
            DispatchQueue.main.async { self?.notchController?.toggle() }
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        GlobalHotkey.unregister()
        daemonLauncher?.stop()
    }
}
