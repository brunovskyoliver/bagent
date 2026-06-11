import AppKit
import SwiftUI

@MainActor
final class PermissionsManager: ObservableObject {

    @Published private(set) var hasFullDiskAccess: Bool = false

    // Probe paths gated by Full Disk Access
    private static let mailProbe  = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Mail/V10/MailData/Envelope Index")
    private static let notesProbe = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite")

    func refresh() {
        hasFullDiskAccess = FileManager.default.isReadableFile(
            atPath: Self.mailProbe.path
        )
    }

    func openPrivacySettings() {
        let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")!
        NSWorkspace.shared.open(url)
    }
}
