import AppKit
import ApplicationServices
import AVFoundation
import CoreGraphics
import SwiftUI

@MainActor
final class PermissionsManager: ObservableObject {

    @Published private(set) var hasFullDiskAccess: Bool = false
    @Published private(set) var hasMicrophoneAccess: Bool = false
    @Published private(set) var hasScreenRecording: Bool = false
    @Published private(set) var hasAccessibility: Bool = false

    // Probe paths gated by Full Disk Access
    private static let mailProbe  = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Mail/V10/MailData/Envelope Index")
    private static let notesProbe = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite")

    func refresh() {
        hasFullDiskAccess = FileManager.default.isReadableFile(
            atPath: Self.mailProbe.path
        )
        hasMicrophoneAccess = AVCaptureDevice.authorizationStatus(for: .audio) == .authorized
        // CGPreflightScreenCaptureAccess probes TCC without prompting the user
        hasScreenRecording = CGPreflightScreenCaptureAccess()
        hasAccessibility   = AXIsProcessTrusted()
    }

    func openPrivacySettings() {
        let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")!
        NSWorkspace.shared.open(url)
    }

    /// Request microphone access (no-op prompt if already determined), then refresh.
    func requestMicrophoneAccess() async {
        if AVCaptureDevice.authorizationStatus(for: .audio) == .notDetermined {
            _ = await AVCaptureDevice.requestAccess(for: .audio)
        }
        refresh()
    }

    func openMicrophoneSettings() {
        let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")!
        NSWorkspace.shared.open(url)
    }

    /// Request Screen Recording access (prompts macOS TCC dialog on first call), then refresh.
    func requestScreenRecording() {
        // CGRequestScreenCaptureAccess() prompts the user if not yet determined.
        CGRequestScreenCaptureAccess()
        refresh()
    }

    func openScreenRecordingSettings() {
        let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")!
        NSWorkspace.shared.open(url)
    }

    /// Request Accessibility access (prompts the system Accessibility dialog), then refresh.
    nonisolated func requestAccessibility() {
        // kAXTrustedCheckOptionPrompt is a non-Sendable C global; call off the actor.
        let key = "AXTrustedCheckOptionPrompt" as CFString
        let opts = [key: true] as CFDictionary
        _ = AXIsProcessTrustedWithOptions(opts)
        Task { @MainActor in self.refresh() }
    }

    func openAccessibilitySettings() {
        let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!
        NSWorkspace.shared.open(url)
    }
}
