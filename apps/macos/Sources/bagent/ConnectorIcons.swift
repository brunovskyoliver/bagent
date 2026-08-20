import SwiftUI

// MARK: - Connector actions surfaced in the notch left wing

/// Connector whose result can be acted on from the notch. Order = display order
/// in the left-wing icon row. Adding a connector later is one new case here.
enum ConnectorKind: String, CaseIterable {
    case whatsapp
    case odoo
    case mail
    case file
}

/// A pending, clickable connector result (found mail / odoo record / whatsapp
/// chat). Lives in the left notch wing until clicked or the next user message.
struct ConnectorAction: Identifiable, Equatable {
    enum Payload {
        case mail(DaemonClient.MailRef)
        case odoo(DaemonClient.OdooRef)
        case whatsapp(DaemonClient.WhatsappRef)
        case file(DaemonClient.FileRef)
    }

    let id = UUID()
    let kind: ConnectorKind
    let payload: Payload

    // Refs aren't Equatable; identity is enough for row diffing.
    static func == (lhs: ConnectorAction, rhs: ConnectorAction) -> Bool {
        lhs.id == rhs.id
    }
}

/// A clicked connector icon mid fly-off, mirroring `CmuxDeparture`.
/// `slotIndex` is the icon's row position at click time so the flight starts
/// from where the icon actually was.
struct ConnectorDeparture: Identifiable {
    let id = UUID()
    let kind: ConnectorKind
    let slotIndex: Int
}

// MARK: - Bell-swing animation shared with the cmux icon

/// One ring (decaying swing) followed by resting beats — extracted from the
/// cmux left-wing icon so connector icons ring the same way.
enum NotchIconSwing {
    static let phases: [Double] = [0, -16, 12, -8, 5, -2, 0, 0, 0]

    static func animation(for angle: Double) -> Animation {
        angle == 0 ? .easeOut(duration: 0.5) : .spring(duration: 0.16, bounce: 0.35)
    }
}

// MARK: - Icon rendering

/// Colorful brand-style connector icon — a rounded-rect badge with a gradient
/// fill and white glyph, sized to sit next to the cmux app icon in the wing.
/// Rings once like a bell when it appears (attention cue), then rests.
struct ConnectorIconView: View {
    let kind: ConnectorKind
    let reduceMotion: Bool
    var size: CGFloat = 18

    @State private var swingTrigger = false

    var body: some View {
        if reduceMotion {
            badge
        } else {
            badge
                .phaseAnimator(NotchIconSwing.phases, trigger: swingTrigger) { view, angle in
                    view.rotationEffect(.degrees(angle), anchor: .top)
                } animation: { angle in
                    NotchIconSwing.animation(for: angle)
                }
                .onAppear { swingTrigger.toggle() }
                // Keep ringing until handled — the timer dies with the view
                // when the action is clicked or cleared.
                .onReceive(Timer.publish(every: 4, on: .main, in: .common).autoconnect()) { _ in
                    swingTrigger.toggle()
                }
        }
    }

    private var badge: some View {
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.24, style: .continuous)
                .fill(LinearGradient(
                    colors: gradientColors,
                    startPoint: .top,
                    endPoint: .bottom
                ))
                .overlay(
                    RoundedRectangle(cornerRadius: size * 0.24, style: .continuous)
                        .stroke(Color.white.opacity(0.15), lineWidth: 0.5)
                )
            glyph
        }
        .frame(width: size, height: size)
    }

    @ViewBuilder
    private var glyph: some View {
        switch kind {
        case .mail:
            Image(systemName: "envelope.fill")
                .font(.system(size: size * 0.52, weight: .semibold))
                .foregroundStyle(.white)
        case .whatsapp:
            Image(systemName: "phone.fill")
                .font(.system(size: size * 0.52, weight: .semibold))
                .foregroundStyle(.white)
        case .odoo:
            Text("o")
                .font(.system(size: size * 0.72, weight: .bold, design: .rounded))
                .foregroundStyle(.white)
                .baselineOffset(size * 0.06)
        case .file:
            Image(systemName: "doc.fill")
                .font(.system(size: size * 0.52, weight: .semibold))
                .foregroundStyle(.white)
        }
    }

    private var gradientColors: [Color] {
        switch kind {
        case .mail:
            return [Color(red: 0.12, green: 0.64, blue: 1.00),
                    Color(red: 0.12, green: 0.44, blue: 0.95)]
        case .whatsapp:
            return [Color(red: 0.15, green: 0.83, blue: 0.40),
                    Color(red: 0.07, green: 0.55, blue: 0.49)]
        case .odoo:
            return [Color(red: 0.53, green: 0.35, blue: 0.48),
                    Color(red: 0.44, green: 0.29, blue: 0.40)]
        case .file:
            return [Color(red: 0.56, green: 0.62, blue: 0.72),
                    Color(red: 0.40, green: 0.46, blue: 0.58)]
        }
    }
}

extension ConnectorKind {
    var accessibilityLabel: String {
        switch self {
        case .mail:     return "Otvoriť mail"
        case .odoo:     return "Otvoriť Odoo záznam"
        case .whatsapp: return "Otvoriť WhatsApp"
        case .file:     return "Zobraziť súbor vo Finderi"
        }
    }
}
