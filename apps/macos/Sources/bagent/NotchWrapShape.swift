import SwiftUI

/// Curved shape that wraps around the physical MacBook notch.
/// Draws a U that hugs the left and right of the notch cutout plus
/// an optional bottom bridge connecting the two wings below the notch.
///
/// All four parameters are animatable so SwiftUI can morph between
/// idle / hover / expanded states smoothly.
struct NotchWrapShape: Shape {

    /// Width of each wing (left and right of the notch).
    var wingWidth: CGFloat
    /// Height of the horizontal bridge drawn below the notch.  0 = no bridge.
    var bridgeHeight: CGFloat
    /// Width of the physical notch gap (fixed, from geometry).
    var notchWidth: CGFloat
    /// Height of the menu bar / notch (fixed, from geometry).
    var notchHeight: CGFloat

    var outerCornerRadius: CGFloat = 10
    var innerCornerRadius: CGFloat = 8

    var animatableData: AnimatablePair<CGFloat, CGFloat> {
        get { AnimatablePair(wingWidth, bridgeHeight) }
        set {
            wingWidth     = newValue.first
            bridgeHeight  = newValue.second
        }
    }

    func path(in rect: CGRect) -> Path {
        // rect spans: (0,0) to (2*wingWidth + notchWidth, notchHeight + bridgeHeight)
        // The shape is open at the top (menu-bar edge) — we only draw the outer
        // boundary that is visible against the menu-bar dark area.

        let w = rect.width   // == 2*wingWidth + notchWidth
        let h = rect.height  // == notchHeight + bridgeHeight
        let oc = min(outerCornerRadius, wingWidth / 2, (notchHeight) / 2)
        let ic = min(innerCornerRadius, wingWidth / 2, notchHeight / 2)
        let bridge = max(0, bridgeHeight)

        var p = Path()

        // Start at top-left outer corner
        p.move(to: CGPoint(x: 0, y: oc))
        // Outer left corner arc (top-left)
        p.addArc(
            center: CGPoint(x: oc, y: oc),
            radius: oc,
            startAngle: .degrees(180),
            endAngle: .degrees(270),
            clockwise: false
        )
        // Top edge of left wing (going right toward notch)
        p.addLine(to: CGPoint(x: wingWidth - ic, y: 0))
        // Inner left notch corner arc (top-right of left wing)
        p.addArc(
            center: CGPoint(x: wingWidth - ic, y: ic),
            radius: ic,
            startAngle: .degrees(270),
            endAngle: .degrees(0),
            clockwise: false
        )
        // Right edge of left wing going down to bridge / bottom of menu bar
        p.addLine(to: CGPoint(x: wingWidth, y: notchHeight + bridge))
        // Bridge bottom going right (only visible when bridgeHeight > 0)
        p.addLine(to: CGPoint(x: wingWidth + notchWidth, y: notchHeight + bridge))
        // Left edge of right wing going up from bridge / bottom of menu bar
        p.addLine(to: CGPoint(x: wingWidth + notchWidth, y: ic))
        // Inner right notch corner arc (top-left of right wing)
        p.addArc(
            center: CGPoint(x: wingWidth + notchWidth + ic, y: ic),
            radius: ic,
            startAngle: .degrees(180),
            endAngle: .degrees(270),
            clockwise: false
        )
        // Top edge of right wing
        p.addLine(to: CGPoint(x: w - oc, y: 0))
        // Outer right corner arc (top-right)
        p.addArc(
            center: CGPoint(x: w - oc, y: oc),
            radius: oc,
            startAngle: .degrees(270),
            endAngle: .degrees(0),
            clockwise: false
        )
        // Right outer edge going down
        p.addLine(to: CGPoint(x: w, y: h))
        // Bottom edge — closes across the full width
        p.addLine(to: CGPoint(x: 0, y: h))
        p.closeSubpath()

        return p
    }
}
