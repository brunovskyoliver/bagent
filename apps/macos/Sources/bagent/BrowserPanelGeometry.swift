import CoreGraphics

struct BrowserPanelGeometry: Equatable {
    let panelSize: CGSize
    let visibleFrame: CGRect
    let dragStripHeight: CGFloat

    init(panelSize: CGSize, visibleFrame: CGRect, dragStripHeight: CGFloat = 26) {
        self.panelSize = panelSize
        self.visibleFrame = visibleFrame
        self.dragStripHeight = max(0, dragStripHeight)
    }

    func frame(anchoredTo pointer: CGPoint) -> CGRect {
        let anchor = CGPoint(
            x: panelSize.width / 2,
            y: max(0, panelSize.height - dragStripHeight / 2)
        )
        let origin = CGPoint(
            x: clamp(pointer.x - anchor.x, lower: visibleFrame.minX, upper: maxOriginX),
            y: clamp(pointer.y - anchor.y, lower: visibleFrame.minY, upper: maxOriginY)
        )
        return CGRect(origin: origin, size: panelSize)
    }

    private var maxOriginX: CGFloat {
        max(visibleFrame.minX, visibleFrame.maxX - panelSize.width)
    }

    private var maxOriginY: CGFloat {
        max(visibleFrame.minY, visibleFrame.maxY - panelSize.height)
    }

    private func clamp(_ value: CGFloat, lower: CGFloat, upper: CGFloat) -> CGFloat {
        min(max(value, lower), upper)
    }

    /// Compact floating size for "Move to top right" — the picture-in-picture
    /// proportions browsers use for popped-out video. Keeps the panel's aspect
    /// ratio and never exceeds the display.
    static func pictureInPictureSize(for size: CGSize, visibleFrame: CGRect,
                                     fraction: CGFloat = 0.26,
                                     minimumWidth: CGFloat = 360,
                                     maximumWidth: CGFloat = 560) -> CGSize {
        let aspect = size.height > 0 ? size.width / size.height : 3.0 / 2.0
        let width = min(max(visibleFrame.width * fraction, minimumWidth),
                        min(maximumWidth, visibleFrame.width))
        let height = min(width / max(aspect, 0.1), visibleFrame.height)
        return CGSize(width: min(width, visibleFrame.width), height: height)
    }

    /// Top-right corner of `visibleFrame`, inset by `margin`, clamped so a
    /// panel wider/taller than the display never spills past its bounds.
    static func topRightOrigin(panelSize: CGSize, visibleFrame: CGRect, margin: CGFloat) -> CGPoint {
        let minX = visibleFrame.minX
        let minY = visibleFrame.minY
        let maxX = max(minX, visibleFrame.maxX - panelSize.width)
        let maxY = max(minY, visibleFrame.maxY - panelSize.height)
        return CGPoint(
            x: min(maxX, max(minX, visibleFrame.maxX - panelSize.width - margin)),
            y: min(maxY, max(minY, visibleFrame.maxY - panelSize.height - margin))
        )
    }
}
