import SwiftUI

/// Single unified fill shape for the notch wrap.
/// Draws ONE rectangle that spans both wings + the notch gap as a solid block.
/// Top corners are FLAT (pressed against the screen/menu-bar edge).
/// Bottom corners are rounded.
///
/// Wings grow outward from the notch edges as `wingWidth` increases,
/// so the animation always radiates from the notch center.
struct NotchWrapShape: Shape {

    /// Width of each wing outward from the notch edge (animatable).
    var wingWidth: CGFloat
    /// Height of the strip below the notch (animatable). 0 = top strip only.
    var bridgeHeight: CGFloat
    /// X of the notch left edge in panel-local coords (= hoverWingWidth, constant).
    let notchOffset: CGFloat
    /// Width of the physical notch gap (constant).
    let notchWidth: CGFloat
    /// Height of the menu-bar / notch strip (constant).
    let notchHeight: CGFloat
    /// Radius applied only to the bottom-left and bottom-right corners.
    var cornerRadius: CGFloat = 10
    /// Depth of the paste-wheel dome hanging below the bottom edge (animatable).
    var bulgeDepth: CGFloat = 0
    /// How much of the dome has been "drawn" left→right (0…1, animatable).
    /// The dome is a circular arc; the sweep frontier reveals it from the
    /// left corner around to the right.
    var bulgeSweep: CGFloat = 0

    var animatableData: AnimatablePair<CGFloat, AnimatablePair<CGFloat, AnimatablePair<CGFloat, CGFloat>>> {
        get {
            AnimatablePair(wingWidth,
                           AnimatablePair(bridgeHeight, AnimatablePair(bulgeDepth, bulgeSweep)))
        }
        set {
            wingWidth = newValue.first
            bridgeHeight = newValue.second.first
            bulgeDepth = newValue.second.second.first
            bulgeSweep = newValue.second.second.second
        }
    }

    func path(in rect: CGRect) -> Path {
        guard wingWidth > 0 else { return Path() }

        let x = notchOffset - wingWidth
        let w = 2 * wingWidth + notchWidth
        let h = notchHeight + max(0, bridgeHeight)
        let r = cornerRadius                      // fixed — never clamped
        let depth = max(0, bulgeDepth)
        let sweep = min(1, max(0, bulgeSweep))

        var p = Path()
        p.move(to: CGPoint(x: x, y: 0))           // top-left, sharp
        p.addLine(to: CGPoint(x: x + w, y: 0))    // top edge

        if depth > 0.5, sweep > 0.01 {
            // Wheel dome: bottom edge is a circular arc spanning corner to
            // corner, revealed left→right by the sweep frontier. Bottom edge
            // is traversed right→left, so draw the flat (not-yet-swept) part
            // first, drop to the frontier point, then follow the ellipse back
            // to the left corner.
            p.addLine(to: CGPoint(x: x + w, y: h))
            let frontier = Self.domePoint(t: sweep, x: x, w: w, h: h, depth: depth)
            p.addLine(to: CGPoint(x: frontier.x, y: h))
            p.addLine(to: frontier)
            let steps = max(2, Int(48 * sweep))
            for i in stride(from: steps - 1, through: 0, by: -1) {
                let t = sweep * CGFloat(i) / CGFloat(steps)
                p.addLine(to: Self.domePoint(t: t, x: x, w: w, h: h, depth: depth))
            }
        } else {
            p.addLine(to: CGPoint(x: x + w, y: h - r))
            p.addArc(
                center: CGPoint(x: x + w - r, y: h - r),
                radius: r, startAngle: .degrees(0), endAngle: .degrees(90), clockwise: false
            )
            p.addLine(to: CGPoint(x: x + r, y: h))
            p.addArc(
                center: CGPoint(x: x + r, y: h - r),
                radius: r, startAngle: .degrees(90), endAngle: .degrees(180), clockwise: false
            )
        }
        p.closeSubpath()
        return p
    }

    /// Point on the dome at fraction `t` (0 = left corner, 0.5 = lowest point,
    /// 1 = right corner). The dome is a true circular arc through both bottom
    /// corners sagging `depth` at its center; `t` is uniform in arc angle, so a
    /// sweep animation traces the circle at constant angular speed.
    static func domePoint(t: CGFloat, x: CGFloat, w: CGFloat, h: CGFloat, depth: CGFloat) -> CGPoint {
        let halfW = w / 2
        let radius = (halfW * halfW + depth * depth) / (2 * depth)
        let halfAngle = asin(min(1, halfW / radius))
        let theta = halfAngle * (2 * t - 1)   // -halfAngle … +halfAngle
        return CGPoint(
            x: x + halfW + radius * sin(theta),
            y: h + depth - radius + radius * cos(theta)
        )
    }

    /// Y-offset of the dome edge below the flat bottom at arc fraction `t` —
    /// lets wheel chips hug the curve.
    static func bulgeOffset(at t: CGFloat, depth: CGFloat, width: CGFloat) -> CGFloat {
        let tt = min(1, max(0, t))
        return domePoint(t: tt, x: 0, w: width, h: 0, depth: depth).y
    }
}

