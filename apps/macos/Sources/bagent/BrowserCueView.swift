import SwiftUI

struct BrowserCueIconView: View {
    let cue: BrowserCue
    let reduceMotion: Bool

    var body: some View {
        ZStack {
            Circle()
                .fill(color)
                .overlay(Circle().stroke(Color.white.opacity(0.22), lineWidth: 0.5))
            // Live agent session: a green ring, readable at 18pt in both the
            // collapsed and the hover-expanded notch.
            if cue.isAgentActive {
                Circle()
                    .stroke(Color.green, lineWidth: 1.6)
                    .frame(width: 17, height: 17)
                    .shadow(color: Color.green.opacity(0.7), radius: 2)
            }
            Image(systemName: cue.state == .attention ? "hand.raised.fill" : "globe")
                .font(.system(size: 9, weight: .semibold))
                .foregroundStyle(.white)
                // Native pulse on the glyph only. The previous
                // `.animation(.repeatForever, value: cue.state)` sat on the
                // whole stack, so a state change animated the glyph's and
                // badge's geometry forever — they drifted out of the circle and
                // never settled.
                .symbolEffect(.pulse, options: .repeating,
                              isActive: !reduceMotion && cue.state == .active)
            Text(badge)
                .font(.system(size: 6, weight: .bold, design: .rounded))
                .foregroundStyle(.white)
                .offset(x: 7, y: 7)
        }
        .frame(width: 18, height: 18)
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: cue.state)
        .accessibilityLabel(String(localized: "browser.cue.accessibility",
                                   defaultValue: "bagent Browser \(cue.label)"))
        .help(cue.origin.map {
            String(localized: "browser.cue.origin", defaultValue: "bagent Browser · \($0)")
        } ?? String(localized: "browser.cue.title", defaultValue: "bagent Browser"))
    }

    private var badge: String {
        String(cue.label.split(separator: "-").first?.prefix(1) ?? "B").uppercased()
    }

    private var color: Color {
        switch cue.state {
        case .steady: return Color.blue.opacity(0.82)
        case .active: return Color.cyan.opacity(0.92)
        case .attention: return Color.orange.opacity(0.92)
        case .detached: return Color.gray.opacity(0.85)
        case .reclaimPending: return Color.purple.opacity(0.9)
        }
    }
}
