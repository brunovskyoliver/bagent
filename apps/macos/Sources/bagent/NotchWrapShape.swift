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

    var animatableData: AnimatablePair<CGFloat, CGFloat> {
        get { AnimatablePair(wingWidth, bridgeHeight) }
        set { wingWidth = newValue.first; bridgeHeight = newValue.second }
    }

    func path(in rect: CGRect) -> Path {
        guard wingWidth > 0 else { return Path() }

        let x = notchOffset - wingWidth
        let w = 2 * wingWidth + notchWidth
        let h = notchHeight + max(0, bridgeHeight)
        let r = cornerRadius                      // fixed — never clamped

        var p = Path()
        p.move(to: CGPoint(x: x, y: 0))           // top-left, sharp
        p.addLine(to: CGPoint(x: x + w, y: 0))    // top edge
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
        p.closeSubpath()
        return p
    }
}

