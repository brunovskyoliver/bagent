import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The five clipboard chips laid along the bulged bottom edge of the notch.
/// Rendered inside the status panel so the wheel is part of the same filled
/// surface the notch morph animates.
struct PasteWheelView: View {
    @ObservedObject var viewModel: ChatViewModel
    let notchOffset: CGFloat
    let notchWidth: CGFloat
    let notchHeight: CGFloat

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var hoveredSlot: Int? = nil
    @State private var hasPinned = false

    private var visibleWidth: CGFloat { notchWidth + 2 * NotchWrapMetrics.wheelWingWidth }
    private var leftEdge: CGFloat { notchOffset - NotchWrapMetrics.wheelWingWidth }

    private static let chipHeight: CGFloat = 52
    private static let chipMaxWidth: CGFloat = 112

    /// Chip center hugging the arc at angular fraction `t`, pulled slightly
    /// toward the center so edge chips stay inside the fill.
    private func chipCenter(slot: Int, count: Int) -> CGPoint {
        let t = (CGFloat(slot) + 0.5) / CGFloat(max(1, count))
        let inward = 0.5 + (t - 0.5) * 0.86
        let edge = NotchWrapShape.domePoint(
            t: inward,
            x: leftEdge,
            w: visibleWidth,
            h: notchHeight + NotchWrapMetrics.wheelBridgeHeight,
            depth: NotchWrapMetrics.wheelBulgeDepth
        )
        return CGPoint(x: edge.x, y: edge.y - Self.chipHeight / 2 - 8)
    }

    var body: some View {
        let items = viewModel.pasteWheelItems
        ZStack(alignment: .topLeading) {
            // Pin trigger — only the visible wheel band, not the oversized panel.
            let bandHeight = NotchWrapMetrics.wheelBridgeHeight + NotchWrapMetrics.wheelBulgeDepth
            Color.clear
                .frame(width: visibleWidth, height: bandHeight)
                .contentShape(Rectangle())
                .onContinuousHover { phase in
                    if case .active = phase, !hasPinned {
                        hasPinned = true
                        viewModel.onPasteWheelPinned?()
                    }
                }
                .position(x: notchOffset + notchWidth / 2,
                          y: notchHeight + bandHeight / 2)
            if items.isEmpty {
                Text("Schránka je prázdna")
                    .font(.system(size: 11))
                    .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                    .position(x: notchOffset + notchWidth / 2,
                              y: notchHeight + NotchWrapMetrics.wheelBridgeHeight / 2 + 10)
            }
            ForEach(Array(items.prefix(ClipboardHistory.capacity).enumerated()), id: \.element.id) { slot, item in
                chip(item: item, slot: slot)
                    .position(chipCenter(slot: slot, count: min(items.count, ClipboardHistory.capacity)))
            }
        }
        .onChange(of: viewModel.pasteWheelActive) { _, active in
            if !active {
                hasPinned = false
                hoveredSlot = nil
            }
        }
    }

    @ViewBuilder
    private func chip(item: ClipboardItem, slot: Int) -> some View {
        let isNewest = slot == 0
        let isFlashed = viewModel.pasteWheelFlashSlot == slot
        let isHovered = hoveredSlot == slot

        // Bare content on the black fill — no boxes, no borders. Selection
        // reads through brightness and scale only.
        VStack(spacing: 4) {
            chipPreview(item)
            Text("\(slot + 1)")
                .font(.system(size: 11, weight: .semibold, design: .rounded))
                .foregroundStyle(isNewest ? Color.white.opacity(0.85) : NotchWrapMetrics.notchTextFaint)
        }
        .padding(.horizontal, 6)
        .frame(height: Self.chipHeight)
        .frame(maxWidth: Self.chipMaxWidth)
        .opacity(isFlashed || isHovered ? 1 : (isNewest ? 0.95 : 0.72))
        .scaleEffect(isFlashed ? 1.16 : (isHovered ? 1.08 : (isNewest ? 1.03 : 1.0)))
        .contentShape(Rectangle())
        .animation(reduceMotion ? nil : .spring(response: 0.22, dampingFraction: 0.7),
                   value: isHovered)
        .animation(reduceMotion ? nil : .spring(response: 0.18, dampingFraction: 0.6),
                   value: isFlashed)
        .onHover { hovering in
            hoveredSlot = hovering ? slot : (hoveredSlot == slot ? nil : hoveredSlot)
        }
        .onTapGesture { viewModel.onPasteWheelChipClicked?(slot) }
        .onDrag {
            viewModel.onPasteWheelDragStarted?()
            return Self.itemProvider(for: item)
        }
        .help(item.label)
        .accessibilityLabel("Schránka \(slot + 1): \(item.label)")
        .accessibilityAddTraits(.isButton)
    }

    @ViewBuilder
    private func chipPreview(_ item: ClipboardItem) -> some View {
        switch item.kind {
        case .text(let preview):
            Text(preview)
                .font(.system(size: 13, design: looksLikeCode(preview) ? .monospaced : .default))
                .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                .lineLimit(2)
                .truncationMode(.tail)
        case .image(let thumbnail):
            Image(nsImage: thumbnail)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(maxHeight: Self.chipHeight - 10)
                .clipShape(RoundedRectangle(cornerRadius: 5, style: .continuous))
        case .files(let urls):
            HStack(spacing: 4) {
                if let first = urls.first {
                    Image(nsImage: NSWorkspace.shared.icon(forFile: first.path))
                        .resizable()
                        .frame(width: 26, height: 26)
                    Text(first.lastPathComponent)
                        .font(.system(size: 12))
                        .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                if urls.count > 1 {
                    Text("+\(urls.count - 1)")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                }
            }
        }
    }

    private func looksLikeCode(_ text: String) -> Bool {
        text.contains("{") || text.contains("</") || text.contains("()")
            || text.hasPrefix("$") || text.contains(" = ")
    }

    /// Drag payload carrying the raw pasteboard representations — the OS
    /// delivers the drop into any app's field natively.
    static func itemProvider(for item: ClipboardItem) -> NSItemProvider {
        if case .files(let urls) = item.kind, let url = urls.first {
            return NSItemProvider(object: url as NSURL)
        }
        let provider = NSItemProvider()
        for (type, data) in item.representations.first ?? [] {
            provider.registerDataRepresentation(
                forTypeIdentifier: type.rawValue,
                visibility: .all
            ) { completion in
                completion(data, nil)
                return nil
            }
        }
        return provider
    }
}
