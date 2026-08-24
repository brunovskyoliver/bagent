import AppKit
import Testing
import WebKit
@testable import bagent

@Suite("Browser cue drag", .serialized)
@MainActor
struct BrowserCueDragRegressionTests {
    @Test("a normal click toggles popup visibility")
    func clickTogglesVisibility() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "click-test")
        defer { coordinator.approveClose(sessionID: sessionID) }

        #expect(coordinator.sessions[sessionID]?.pageInfo.visibility == .hidden)
        coordinator.toggleCue(sessionID)
        #expect(coordinator.sessions[sessionID]?.pageInfo.visibility == .popup)
        coordinator.toggleCue(sessionID)
        #expect(coordinator.sessions[sessionID]?.pageInfo.visibility == .hidden)
    }

    @Test("crossing the drag boundary suppresses click recognition")
    func dragSuppressesClickToggle() {
        var state = BrowserCueDragState()
        let start = CGPoint(x: 9, y: 9)

        #expect(state.update(startLocation: start, location: start) == .waiting)
        #expect(state.update(startLocation: start, location: CGPoint(x: 15, y: 9)) == .began)
        #expect(state.end(startLocation: start, location: CGPoint(x: 24, y: 9)) == .ended)
    }

    @Test("leaving the cue hit region starts a drag")
    func leavingCueStartsDrag() {
        var state = BrowserCueDragState(threshold: 100)
        let start = CGPoint(x: 9, y: 9)

        #expect(state.update(startLocation: start, location: CGPoint(x: 18.5, y: 9)) == .began)
    }

    @Test("drag reveals the selected panel before release")
    func dragRevealsPanel() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "reveal-test")
        defer { coordinator.approveClose(sessionID: sessionID) }

        coordinator.dragCue(sessionID, phase: .began, location: CGPoint(x: 640, y: 720))

        #expect(coordinator.sessions[sessionID]?.pageInfo.visibility == .popup)
        #expect(coordinator.sessions[sessionID]?.windowController.panel.isVisible == true)
    }

    @Test("repeated drag updates move the panel")
    func repeatedDragUpdatesMovePanel() {
        let geometry = BrowserPanelGeometry(
            panelSize: CGSize(width: 400, height: 300),
            visibleFrame: CGRect(x: 100, y: 100, width: 1_400, height: 900)
        )
        let first = geometry.frame(anchoredTo: CGPoint(x: 500, y: 650))
        let second = geometry.frame(anchoredTo: CGPoint(x: 950, y: 420))

        #expect(first != second)
        #expect(first.midX == 500)
        #expect(second.midX == 950)
    }

    @Test("release keeps the popup visible at its final position")
    func releaseKeepsPopupVisible() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "release-test")
        defer { coordinator.approveClose(sessionID: sessionID) }

        coordinator.dragCue(sessionID, phase: .began, location: CGPoint(x: 640, y: 720))
        coordinator.dragCue(sessionID, phase: .changed, location: CGPoint(x: 700, y: 680))
        coordinator.dragCue(sessionID, phase: .ended, location: CGPoint(x: 760, y: 640))

        #expect(coordinator.sessions[sessionID]?.pageInfo.visibility == .popup)
        #expect(coordinator.sessions[sessionID]?.windowController.panel.isVisible == true)
    }

    @Test("panel geometry clamps to the selected display visible bounds")
    func coordinatesClampToSelectedDisplay() {
        let selectedDisplay = CGRect(x: 1_000, y: 100, width: 1_200, height: 800)
        let geometry = BrowserPanelGeometry(
            panelSize: CGSize(width: 400, height: 300),
            visibleFrame: selectedDisplay
        )

        let frame = geometry.frame(anchoredTo: CGPoint(x: 100, y: 2_000))

        #expect(frame.minX == selectedDisplay.minX)
        #expect(frame.maxX <= selectedDisplay.maxX)
        #expect(frame.maxY == selectedDisplay.maxY)
        #expect(frame.minY >= selectedDisplay.minY)
    }

    @Test("the selected cue moves only its own session panel")
    func selectedCueControlsOnlyItsOwnSession() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let firstID = try await openSession(coordinator, connectionID: "first")
        let secondID = try await openSession(coordinator, connectionID: "second")
        defer {
            coordinator.approveClose(sessionID: firstID)
            coordinator.approveClose(sessionID: secondID)
        }

        let secondInitialFrame = try #require(coordinator.sessions[secondID]?.windowController.panel.frame)
        coordinator.dragCue(firstID, phase: .began, location: CGPoint(x: 640, y: 720))
        coordinator.dragCue(firstID, phase: .changed, location: CGPoint(x: 760, y: 640))
        coordinator.dragCue(firstID, phase: .ended, location: CGPoint(x: 820, y: 580))

        let firstSession = try #require(coordinator.sessions[firstID])
        let secondSession = try #require(coordinator.sessions[secondID])
        #expect(firstSession.pageInfo.visibility == .popup)
        #expect(firstSession.windowController.panel.frame != secondInitialFrame)
        #expect(secondSession.pageInfo.visibility == .hidden)
        #expect(secondSession.windowController.panel.frame == secondInitialFrame)
    }

    @Test("agent semantic interaction never reveals a hidden popup")
    func semanticInteractionDoesNotRevealPopup() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "semantic-test")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])

        do {
            _ = try await session.interact(BrowserAction(type: "click", x: 1, y: 1))
            Issue.record("A hidden semantic interaction unexpectedly succeeded")
        } catch let failure as BrowserFailure {
            #expect(failure.code == .visibleInteractionRequired)
        }

        #expect(session.pageInfo.visibility == .hidden)
        #expect(session.windowController.panel.isVisible == false)
    }

    @Test("cue drag keeps the pointer on the panel's top strip")
    func dragDoesNotUsePanelBottomLeftAsPointer() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "drag-test")
        let pointer = CGPoint(x: 640, y: 720)

        coordinator.dragCue(sessionID, phase: .began, location: pointer)

        let frame = try #require(coordinator.sessions[sessionID]?.windowController.panel.frame)
        #expect(frame.midX == pointer.x)
        #expect(frame.maxY - 13 == pointer.y)
    }

    @Test("a live cue widens the collapsed notch's left wing and stays inside the hit region")
    func collapsedCueWidensRequiredWingWidth() {
        #expect(NotchLeftWingLayout.requiredWingWidth(count: 0) == 0)
        let oneWing = NotchLeftWingLayout.requiredWingWidth(count: 1)
        #expect(oneWing > 0)
        // Icon row width plus the two-sided inset must fully contain the icon:
        // the panel's contentShape is the notch clip shape sized by this wing,
        // so a wing narrower than the icon would clip the cue's hit region.
        #expect(oneWing >= NotchLeftWingLayout.iconSize + 2 * NotchLeftWingLayout.rowInset)
    }

    @Test("adding a second cue changes the wing width and both stay independently sized")
    func multipleCuesChangeWingWidth() {
        let one = NotchLeftWingLayout.requiredWingWidth(count: 1)
        let two = NotchLeftWingLayout.requiredWingWidth(count: 2)
        #expect(two > one)

        // Icons always sit side by side — never as an overlapping deck.
        for count in 2...4 {
            let width = NotchLeftWingLayout.rowWidth(count: count)
            #expect(width == CGFloat(count) * NotchLeftWingLayout.iconSize
                    + CGFloat(count - 1) * NotchLeftWingLayout.iconSpacing)
            // Adjacent icon rects must not intersect.
            let step = NotchLeftWingLayout.iconSize + NotchLeftWingLayout.iconSpacing
            let frames = (0..<count).map {
                CGRect(x: CGFloat($0) * step, y: 0,
                       width: NotchLeftWingLayout.iconSize, height: NotchLeftWingLayout.iconSize)
            }
            for (a, b) in zip(frames, frames.dropFirst()) { #expect(!a.intersects(b)) }
        }
    }

    @Test("hiding a session from the panel control keeps the session and cue alive")
    func collapseControlKeepsSessionAlive() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "collapse-test")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])
        try session.show(focus: false)
        #expect(session.pageInfo.visibility == .popup)

        session.windowController.onCollapseRequested?()

        #expect(session.pageInfo.visibility == .hidden)
        #expect(coordinator.sessions[sessionID] != nil)
        #expect(coordinator.cues.contains { $0.id == sessionID })

        // Reopening from the cue restores the same panel/session.
        coordinator.toggleCue(sessionID)
        #expect(session.pageInfo.visibility == .popup)
    }

    @Test("move to top right shrinks to a picture-in-picture popup and stays on the display")
    func topRightUsesPictureInPictureSize() {
        let primary = CGRect(x: 0, y: 0, width: 1_512, height: 945)
        let secondary = CGRect(x: 1_512, y: 200, width: 1_920, height: 1_080)
        let full = CGSize(width: 960, height: 640)

        for visibleFrame in [primary, secondary] {
            let size = BrowserPanelGeometry.pictureInPictureSize(for: full, visibleFrame: visibleFrame)
            #expect(size.width < full.width)
            #expect(size.height < full.height)
            // Aspect ratio preserved.
            #expect(abs(size.width / size.height - full.width / full.height) < 0.01)
            #expect(size.width <= visibleFrame.width)
            #expect(size.height <= visibleFrame.height)

            let origin = BrowserPanelGeometry.topRightOrigin(panelSize: size, visibleFrame: visibleFrame, margin: 12)
            let frame = CGRect(origin: origin, size: size)
            #expect(frame.maxX == visibleFrame.maxX - 12)
            #expect(frame.maxY == visibleFrame.maxY - 12)
            #expect(visibleFrame.contains(frame))
        }
    }

    @Test("a panel larger than the display still lands fully inside it")
    func topRightPlacementClamps() {
        let primary = CGRect(x: 0, y: 0, width: 1_512, height: 945)
        let secondary = CGRect(x: 1_512, y: 200, width: 1_920, height: 1_080)
        let panelSize = CGSize(width: 960, height: 640)

        for visibleFrame in [primary, secondary] {
            let origin = BrowserPanelGeometry.topRightOrigin(panelSize: panelSize, visibleFrame: visibleFrame, margin: 12)
            let frame = CGRect(origin: origin, size: panelSize)
            #expect(frame.maxX <= visibleFrame.maxX)
            #expect(frame.maxY <= visibleFrame.maxY)
            #expect(frame.minX >= visibleFrame.minX)
            #expect(frame.minY >= visibleFrame.minY)
            #expect(frame.maxX == visibleFrame.maxX - 12)
            #expect(frame.maxY == visibleFrame.maxY - 12)
        }
    }

    @Test("a panel control button click is excluded from manual-input control preemption")
    func controlButtonClickDoesNotRevokeControlLease() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "control-lease-test")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])

        var manualInputFired = false
        session.windowController.onManualInput = { manualInputFired = true }
        session.windowController.onCollapseRequested?()

        #expect(manualInputFired == false)
    }

    // MARK: - Cold first mouse (the live failure: cue unusable until ⌥Space)

    @Test("the cue's own hit view accepts the very first mouse of an inactive app")
    func cueHitViewAcceptsFirstMouse() {
        _ = NSApplication.shared
        #expect(BrowserCueHitView().acceptsFirstMouse(for: nil) == true)
    }

    @Test("the collapsed notch hit-tests a live cue to its AppKit interaction view, and dragging it never expands the notch")
    func collapsedNotchHitTestsCueInteractionView() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "hit-test")
        defer { coordinator.approveClose(sessionID: sessionID) }

        let viewModel = ChatViewModel(startMonitoring: false)
        let controller = NotchWindowController(chatViewModel: viewModel, browserCoordinator: coordinator)
        let panel = controller.statusPanelForTesting
        panel.orderFront(nil)
        defer { panel.orderOut(nil) }
        let contentView = try #require(panel.contentView)
        for _ in 0..<8 {
            contentView.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
            if descendants(of: contentView).contains(where: { $0 is BrowserCueHitView }) { break }
        }

        let hitView = try #require(descendants(of: contentView).compactMap { $0 as? BrowserCueHitView }.first)
        let center = hitView.convert(CGPoint(x: hitView.bounds.midX, y: hitView.bounds.midY), to: nil)
        #expect(contentView.hitTest(center) === hitView)
        #expect(hitView.acceptsFirstMouse(for: nil) == true)
        // The overlay is exactly the cue, so neighbouring wing icons keep their
        // own clicks.
        #expect(hitView.bounds.size == CGSize(width: NotchLeftWingLayout.iconSize,
                                              height: NotchLeftWingLayout.iconSize))

        // Covering the cue with an NSView must not cost it the SwiftUI context
        // menu (close / approve submission / reclaim live there): the hit view
        // steps aside for the events that open it.
        func event(_ type: NSEvent.EventType, _ flags: NSEvent.ModifierFlags) throws -> NSEvent {
            try #require(NSEvent.mouseEvent(
                with: type,
                location: center,
                modifierFlags: flags,
                timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: panel.windowNumber,
                context: nil,
                eventNumber: 0,
                clickCount: 1,
                pressure: 1
            ))
        }
        #expect(BrowserCueHitView.opensContextMenu(try event(.rightMouseDown, [])) == true)
        #expect(BrowserCueHitView.opensContextMenu(try event(.leftMouseDown, .control)) == true)
        #expect(BrowserCueHitView.opensContextMenu(try event(.leftMouseDown, [])) == false)
        #expect(BrowserCueHitView.opensContextMenu(nil) == false)

        // The cue stays hit-testable in every mode that shows it. Inline
        // surfaces deliberately hide it (⌥Space leaves just the input field).
        // The notch mode is projected from authoritative Work state, so each mode
        // is entered through its real driver rather than by assigning the derived
        // properties the Stage 8 cleanup removed.
        controller.collapse()
        contentView.layoutSubtreeIfNeeded()
        try? await Task.sleep(for: .milliseconds(20))
        let collapsedMode = viewModel.notchInteractionMode
        let modeHitView = try #require(descendants(of: contentView).compactMap { $0 as? BrowserCueHitView }.first)
        let modeCenter = modeHitView.convert(
            CGPoint(x: modeHitView.bounds.midX, y: modeHitView.bounds.midY), to: nil)
        #expect(contentView.hitTest(modeCenter) === modeHitView, "cue unreachable in \(collapsedMode)")
        for enter in [controller.presentInputOnly, controller.presentOutputChat] {
            enter()
            contentView.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(40))
            let mode = viewModel.notchInteractionMode
            #expect(descendants(of: contentView).compactMap { $0 as? BrowserCueHitView }.isEmpty,
                    "the cue must step aside for the inline surface in \(mode)")
        }

        controller.collapse()
        contentView.layoutSubtreeIfNeeded()
        try? await Task.sleep(for: .milliseconds(40))

        // Drive the exact production sequence the cue owns.
        var phases: [BrowserCueDragPhase] = []
        var clicks = 0
        hitView.onClick = { clicks += 1 }
        hitView.onDragPhase = { phase, location in
            phases.append(phase)
            coordinator.dragCue(sessionID, phase: phase, location: location)
        }
        let start = CGPoint(x: hitView.bounds.midX, y: hitView.bounds.midY)
        var state = hitView.beginTracking(at: start)
        hitView.track(.leftMouseDragged, local: CGPoint(x: start.x + 40, y: start.y),
                      screen: CGPoint(x: 640, y: 720), from: start, state: &state)
        hitView.track(.leftMouseDragged, local: CGPoint(x: start.x + 90, y: start.y + 30),
                      screen: CGPoint(x: 700, y: 660), from: start, state: &state)
        hitView.track(.leftMouseUp, local: CGPoint(x: start.x + 120, y: start.y + 50),
                      screen: CGPoint(x: 760, y: 620), from: start, state: &state)

        #expect(phases == [.began, .changed, .ended])
        #expect(clicks == 0)
        // The notch input surface stays collapsed for the whole drag.
        #expect(viewModel.notchInteractionMode == .collapsed)
        #expect(controller.isNotchInteractionShowing == false)
        #expect(coordinator.sessions[sessionID]?.pageInfo.visibility == .popup)
    }

    @Test("a short press inside the cue emits a click and no drag phase")
    func cueHitViewShortPressIsAClick() {
        _ = NSApplication.shared
        let hitView = BrowserCueHitView(frame: CGRect(x: 0, y: 0, width: 18, height: 18))
        var phases: [BrowserCueDragPhase] = []
        var clicks = 0
        hitView.onClick = { clicks += 1 }
        hitView.onDragPhase = { phase, _ in phases.append(phase) }

        let start = CGPoint(x: 9, y: 9)
        var state = hitView.beginTracking(at: start)
        hitView.track(.leftMouseUp, local: CGPoint(x: 10, y: 10), screen: CGPoint(x: 500, y: 500),
                      from: start, state: &state)

        #expect(clicks == 1)
        #expect(phases.isEmpty)
    }

    @Test("drag phases carry screen coordinates, not view-local ones")
    func cueHitViewEmitsScreenCoordinates() {
        _ = NSApplication.shared
        let hitView = BrowserCueHitView(frame: CGRect(x: 0, y: 0, width: 18, height: 18))
        var locations: [CGPoint] = []
        hitView.onDragPhase = { _, location in locations.append(location) }

        let start = CGPoint(x: 9, y: 9)
        var state = hitView.beginTracking(at: start)
        hitView.track(.leftMouseDragged, local: CGPoint(x: 40, y: 9), screen: CGPoint(x: 1_204, y: 806),
                      from: start, state: &state)

        #expect(locations == [CGPoint(x: 1_204, y: 806)])
    }

    @Test("the Browser Panel stays on screen while bagent is not the frontmost app")
    func browserPanelSurvivesDeactivation() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "deactivate-test")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let panel = try #require(coordinator.sessions[sessionID]?.windowController.panel)

        // NSPanel defaults this to true unless it is a nonactivating panel, and
        // this one isn't: left at the default the panel is invisible in its
        // normal state (bagent in the background) and a cue drag shows nothing.
        #expect(panel.hidesOnDeactivate == false)

        // The cue drag must reveal it without focusing/activating bagent.
        coordinator.dragCue(sessionID, phase: .began, location: CGPoint(x: 640, y: 720))
        #expect(panel.isVisible == true)
        #expect(panel.isKeyWindow == false)
    }

    // MARK: - Drag the panel back into the notch

    @Test("dropping the panel on the notch collapses it into its cue and keeps the session alive")
    func droppingPanelOnNotchCollapsesIt() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "drop-in-notch")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])
        let notch = CGRect(x: 500, y: 900, width: 300, height: 40)
        coordinator.notchDropZoneProvider = { notch }
        try session.show(focus: false)
        #expect(session.pageInfo.visibility == .popup)

        coordinator.panelDrag(sessionID, phase: .began, at: CGPoint(x: 640, y: 400))
        #expect(coordinator.isNotchDropTargeted == false)
        coordinator.panelDrag(sessionID, phase: .changed, at: CGPoint(x: notch.midX, y: notch.midY))
        #expect(coordinator.isNotchDropTargeted == true)
        coordinator.panelDrag(sessionID, phase: .ended, at: CGPoint(x: notch.midX, y: notch.midY))

        #expect(coordinator.isNotchDropTargeted == false)
        #expect(session.pageInfo.visibility == .hidden)
        #expect(coordinator.sessions[sessionID] != nil)
        #expect(coordinator.cues.contains { $0.id == sessionID })
    }

    @Test("pushing the panel's top edge into the notch counts as a drop, not just the cursor")
    func panelOverlapCountsAsNotchDrop() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "overlap-drop")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])
        let notch = CGRect(x: 500, y: 900, width: 300, height: 40)
        coordinator.notchDropZoneProvider = { notch }
        try session.show(focus: false)

        // Grab the top strip, then push the panel up so its top edge slides
        // under the menu bar while the cursor stays below the notch band.
        session.windowController.setFrame(CGRect(x: 520, y: 300, width: 260, height: 620))
        coordinator.panelDrag(sessionID, phase: .began, at: CGPoint(x: 640, y: 910))
        // Grabbed 10pt below the panel's top edge, so a cursor just under the
        // notch band still puts the panel's top inside it.
        let cursorBelow = CGPoint(x: 640, y: 895)
        #expect(!notch.contains(cursorBelow))

        coordinator.panelDrag(sessionID, phase: .changed, at: cursorBelow)
        #expect(coordinator.isNotchDropTargeted == true)
        coordinator.panelDrag(sessionID, phase: .ended, at: cursorBelow)

        #expect(session.pageInfo.visibility == .hidden)
        #expect(coordinator.cues.contains { $0.id == sessionID })
    }

    @Test("the drop zone grows with the expanded notch")
    func dropZoneGrowsWhenNotchExpands() {
        _ = NSApplication.shared
        let viewModel = ChatViewModel(startMonitoring: false)
        let controller = NotchWindowController(chatViewModel: viewModel)
        let collapsed = controller.notchDropZone

        // The notch mode is projected from Work state, so drive it through the
        // controller rather than assigning the derived property.
        controller.presentInputOnly()
        let expanded = controller.notchDropZone

        #expect(expanded.width > collapsed.width)
        #expect(expanded.height > collapsed.height)
        #expect(expanded.contains(CGPoint(x: collapsed.midX, y: collapsed.midY)))
    }

    @Test("dropping the panel anywhere else leaves it open")
    func droppingPanelOutsideNotchKeepsItOpen() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "drop-elsewhere")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])
        coordinator.notchDropZoneProvider = { CGRect(x: 500, y: 900, width: 300, height: 40) }
        try session.show(focus: false)

        coordinator.panelDrag(sessionID, phase: .began, at: CGPoint(x: 200, y: 200))
        coordinator.panelDrag(sessionID, phase: .ended, at: CGPoint(x: 200, y: 200))

        #expect(session.pageInfo.visibility == .popup)
        #expect(coordinator.isNotchDropTargeted == false)
    }

    @Test("entering the notch previews the collapse, leaving springs the panel back under the cursor")
    func dropPreviewShrinksAndRestores() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "drop-preview")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])
        let controller = session.windowController
        let notch = CGRect(x: 500, y: 900, width: 300, height: 40)
        coordinator.notchDropZoneProvider = { notch }
        try session.show(focus: false)
        controller.setFrame(CGRect(x: 100, y: 100, width: 960, height: 640))
        let fullSize = controller.panel.frame.size

        let cueRect = CGRect(x: notch.midX - 9, y: notch.midY - 9, width: 18, height: 18)
        coordinator.updateCueRect(sessionID, cueRect)

        coordinator.panelDrag(sessionID, phase: .began, at: CGPoint(x: 300, y: 500))
        coordinator.panelDrag(sessionID, phase: .changed, at: CGPoint(x: notch.midX, y: notch.midY))
        #expect(controller.isDropPreviewActive == true)
        // Following the cursor stops while it is collapsed, so the animation isn't fought.
        #expect(controller.suspendsDragFollowing == true)
        try? await Task.sleep(for: .milliseconds(400))
        // Collapsed the whole way into the cue, not parked half-scale on screen.
        #expect(controller.panel.frame.width < fullSize.width * 0.2)
        #expect(abs(controller.panel.frame.midX - cueRect.midX) < 1)
        #expect(abs(controller.panel.frame.midY - cueRect.midY) < 1)
        #expect(controller.panel.alphaValue < 0.1)

        // Back out of the notch: full size again, under the cursor.
        coordinator.panelDrag(sessionID, phase: .changed, at: CGPoint(x: 300, y: 400))
        #expect(controller.isDropPreviewActive == false)
        try? await Task.sleep(for: .milliseconds(500))
        #expect(controller.panel.frame.size == fullSize)
        #expect(controller.panel.alphaValue == 1)
        #expect(controller.suspendsDragFollowing == false)
        #expect(session.pageInfo.visibility == .popup)
    }

    @Test("releasing inside the notch confirms the collapse and the panel reopens at full size")
    func releasingInsideNotchConfirmsCollapse() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "drop-confirm")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])
        let controller = session.windowController
        let notch = CGRect(x: 500, y: 900, width: 300, height: 40)
        coordinator.notchDropZoneProvider = { notch }
        try session.show(focus: false)
        controller.setFrame(CGRect(x: 100, y: 100, width: 960, height: 640))
        let fullSize = controller.panel.frame.size

        coordinator.panelDrag(sessionID, phase: .began, at: CGPoint(x: 300, y: 500))
        coordinator.panelDrag(sessionID, phase: .changed, at: CGPoint(x: notch.midX, y: notch.midY))
        coordinator.panelDrag(sessionID, phase: .ended, at: CGPoint(x: notch.midX, y: notch.midY))
        try? await Task.sleep(for: .milliseconds(600))

        #expect(session.pageInfo.visibility == .hidden)
        #expect(controller.isDropPreviewActive == false)
        // Reopening must not restore the shrunken preview size.
        #expect(controller.panel.frame.size == fullSize)
        coordinator.toggleCue(sessionID)
        try? await Task.sleep(for: .milliseconds(500))
        #expect(controller.panel.frame.size == fullSize)
    }

    @Test("the cue's screen rect reaches the panel so it can grow out of and collapse into it")
    func cueRectReachesTheWindowController() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "cue-rect")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let rect = CGRect(x: 700, y: 1_100, width: 18, height: 18)

        coordinator.updateCueRect(sessionID, rect)

        #expect(coordinator.sessions[sessionID]?.windowController.cueScreenRect == rect)
    }

    @Test("show, hide and move-to-top-right animations never revoke the agent's Control Lease")
    func windowAnimationsDoNotCountAsManualInput() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "animation-lease")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])
        coordinator.updateCueRect(sessionID, CGRect(x: 700, y: 1_100, width: 18, height: 18))

        var manualInput = false
        var manualResize = false
        session.windowController.onManualInput = { manualInput = true }
        session.windowController.onManualResize = { manualResize = true }

        try session.show(focus: false)
        session.windowController.moveToTopRight()
        session.hide()
        try? await Task.sleep(for: .milliseconds(700))

        #expect(manualInput == false)
        #expect(manualResize == false)
    }

    @Test("the notch drop zone covers the collapsed notch")
    func notchDropZoneCoversTheNotch() {
        _ = NSApplication.shared
        let controller = NotchWindowController(chatViewModel: ChatViewModel(startMonitoring: false))
        let zone = controller.notchDropZone
        #expect(zone.width > 0)
        #expect(zone.height > 0)
        if let screen = NSScreen.main {
            #expect(zone.maxY <= screen.frame.maxY + 1)
            #expect(zone.midX > screen.frame.minX)
            #expect(zone.midX < screen.frame.maxX)
        }
    }

    // MARK: - Left wing composition

    @Test("the hover status icon never overlaps the cue row, at any cue count, stacked or fanned")
    func statusIconNeverCollidesWithCueRow() {
        let notchOffset = NotchWrapMetrics.maxWingWidth
        let size = NotchLeftWingLayout.iconSize

        for count in 1...4 {
            let wing = max(NotchWrapMetrics.hoverWingWidth,
                           NotchLeftWingLayout.requiredWingWidth(count: count, statusIcon: true))
            let rowWidth = NotchLeftWingLayout.rowWidth(count: count)
            let rowRight = notchOffset - NotchLeftWingLayout.rowInset
            let row = CGRect(x: rowRight - rowWidth, y: 0, width: rowWidth, height: size)
            let centerX = NotchLeftWingLayout.statusIconCenterX(
                notchOffset: notchOffset, wingWidth: wing, count: count)
            let statusIcon = CGRect(x: centerX - size / 2, y: 0, width: size, height: size)

            #expect(!statusIcon.intersects(row), "count=\(count)")
            // Both stay inside the black pill.
            #expect(statusIcon.minX >= notchOffset - wing, "count=\(count)")
            #expect(row.maxX <= notchOffset)
        }
    }

    @Test("without cues the status icon keeps its centred position")
    func statusIconCenteredWhenNoCues() {
        let notchOffset = NotchWrapMetrics.maxWingWidth
        let wing = NotchWrapMetrics.hoverWingWidth
        #expect(NotchLeftWingLayout.requiredWingWidth(count: 0, statusIcon: true) == 0)
        #expect(NotchLeftWingLayout.statusIconCenterX(notchOffset: notchOffset, wingWidth: wing,
                                                      count: 0)
                == notchOffset - wing / 2)
    }

    @Test("the wing reserves room for the cue row and the status icon together")
    func wingReservesBothSlots() {
        for count in 1...4 {
            let withIcon = NotchLeftWingLayout.requiredWingWidth(count: count, statusIcon: true)
            let withoutIcon = NotchLeftWingLayout.requiredWingWidth(count: count, statusIcon: false)
            #expect(withIcon > withoutIcon)
            #expect(withIcon - withoutIcon >= NotchLeftWingLayout.iconSize)
        }
    }

    // MARK: - ⌥-click and the active-agent indicator

    @Test("an MCP call marks its session as actively driven and the cue shows it")
    func agentActivityMarksTheCue() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "activity")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])

        #expect(session.isAgentActive == true)
        #expect(coordinator.cues.first { $0.id == sessionID }?.isAgentActive == true)

        // The mark expires once the window lapses.
        session.markAgentActivity(at: Date().addingTimeInterval(-BrowserSession.agentActivityWindow - 1))
        #expect(session.isAgentActive == false)
    }

    @Test("⌥-click destroys an idle Browser Session")
    func optionClickClosesIdleSession() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "option-close")
        let session = try #require(coordinator.sessions[sessionID])
        session.markAgentActivity(at: Date().addingTimeInterval(-BrowserSession.agentActivityWindow - 1))

        coordinator.optionClickCue(sessionID)

        #expect(coordinator.sessions[sessionID] == nil)
        #expect(!coordinator.cues.contains { $0.id == sessionID })
    }

    @Test("⌥-click on a session its agent is using shakes the cue instead of closing it")
    func optionClickOnActiveSessionShakes() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "option-busy")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])
        #expect(session.isAgentActive == true)

        coordinator.optionClickCue(sessionID)

        #expect(coordinator.sessions[sessionID] != nil)
        #expect(coordinator.cues.contains { $0.id == sessionID })
        #expect(coordinator.cueShakes[sessionID] == 1)

        // Repeats replay the shake rather than accumulating a pending close.
        coordinator.optionClickCue(sessionID)
        #expect(coordinator.cueShakes[sessionID] == 2)
        #expect(coordinator.sessions[sessionID] != nil)
    }

    @Test("⌥-click only ever touches its own session")
    func optionClickTargetsOneSession() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let firstID = try await openSession(coordinator, connectionID: "opt-first")
        let secondID = try await openSession(coordinator, connectionID: "opt-second")
        defer { coordinator.approveClose(sessionID: secondID) }
        coordinator.sessions[firstID]?.markAgentActivity(
            at: Date().addingTimeInterval(-BrowserSession.agentActivityWindow - 1))

        coordinator.optionClickCue(firstID)

        #expect(coordinator.sessions[firstID] == nil)
        #expect(coordinator.sessions[secondID] != nil)
    }

    @Test("⌥-click over a cue is claimed by the cue even while the busy shake offsets it")
    func cuePointGuardSurvivesTheShake() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "point-guard")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let rect = CGRect(x: 700, y: 1_100, width: 18, height: 18)
        coordinator.updateCueRect(sessionID, rect)

        #expect(coordinator.isPointOverCue(CGPoint(x: rect.midX, y: rect.midY)))
        // The shake is a render transform of up to 3pt, so the padded rect has
        // to keep claiming clicks that land on the drawn icon.
        #expect(coordinator.isPointOverCue(CGPoint(x: rect.minX - 3, y: rect.midY)))
        #expect(coordinator.isPointOverCue(CGPoint(x: rect.maxX + 3, y: rect.midY)))
        #expect(!coordinator.isPointOverCue(CGPoint(x: rect.maxX + 40, y: rect.midY)))

        // A closed session stops claiming its old position.
        coordinator.approveClose(sessionID: sessionID)
        #expect(!coordinator.isPointOverCue(CGPoint(x: rect.midX, y: rect.midY)))
    }

    @Test("the active marker lapses once the agent stops calling")
    func activityMarkerLapses() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "lapse")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])
        #expect(session.isAgentActive == true)

        // Wait out the window with no further calls.
        try? await Task.sleep(for: .seconds(BrowserSession.agentActivityWindow + 1))
        #expect(session.isAgentActive == false)
        #expect(coordinator.cues.first { $0.id == sessionID }?.isAgentActive == false)
    }


    // MARK: - Cue hover preview

    @Test("the hover preview sits under the cursor and stays on the cursor's display")
    func cuePreviewGeometryStaysOnDisplay() {
        let primary = CGRect(x: 0, y: 0, width: 1_512, height: 945)
        let secondary = CGRect(x: 1_512, y: 200, width: 1_920, height: 1_080)

        for visibleFrame in [primary, secondary] {
            let cursor = CGPoint(x: visibleFrame.midX, y: visibleFrame.maxY - 4)
            let frame = BrowserCuePreviewGeometry.frame(under: cursor, visibleFrame: visibleFrame)
            #expect(frame.maxY <= cursor.y - BrowserCuePreviewGeometry.cursorGap)
            #expect(abs(frame.midX - cursor.x) < 0.5)
            #expect(visibleFrame.contains(frame))

            // Clamped horizontally at both edges.
            for x in [visibleFrame.minX, visibleFrame.maxX] {
                let clamped = BrowserCuePreviewGeometry.frame(
                    under: CGPoint(x: x, y: visibleFrame.maxY - 4), visibleFrame: visibleFrame)
                #expect(visibleFrame.contains(clamped))
            }
        }
    }

    @Test("hovering a cue shows its own preview and a drag replaces it with the panel")
    func cueHoverPreviewFollowsTheHoveredSession() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "preview-test")
        defer { coordinator.approveClose(sessionID: sessionID) }

        coordinator.previewCue(sessionID, at: CGPoint(x: 640, y: 900))
        #expect(coordinator.isCuePreviewVisible == true)

        coordinator.hideCuePreview()
        #expect(coordinator.isCuePreviewVisible == false)

        // Starting a drag must not leave the card floating over the panel.
        coordinator.previewCue(sessionID, at: CGPoint(x: 640, y: 900))
        coordinator.dragCue(sessionID, phase: .began, location: CGPoint(x: 640, y: 720))
        #expect(coordinator.isCuePreviewVisible == false)
        #expect(coordinator.sessions[sessionID]?.windowController.panel.isVisible == true)
    }

    @Test("an unknown cue never opens a preview")
    func cuePreviewIgnoresUnknownSessions() {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        coordinator.previewCue(UUID(), at: CGPoint(x: 100, y: 100))
        #expect(coordinator.isCuePreviewVisible == false)
    }

    // MARK: - Browser Panel controls

    @Test("the panel controls sit above the web view and the drag strip in hit-test order")
    func controlClusterIsAboveWebViewAndDragStrip() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "controls-hit-order")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let panel = try #require(coordinator.sessions[sessionID]?.windowController.panel)
        let contentView = try #require(panel.contentView)
        contentView.layoutSubtreeIfNeeded()

        let cluster = try #require(descendants(of: contentView).compactMap { $0 as? BrowserPanelControlCluster }.first)
        for button in cluster.buttons {
            let center = button.convert(CGPoint(x: button.bounds.midX, y: button.bounds.midY), to: nil)
            #expect(contentView.hitTest(center) === button)
        }
        // Capsule padding still belongs to the chrome, never the page.
        // The capsule must sit fully inside the panel: centring it on the drag
        // strip pushed its top edge past the panel's top and clipped it.
        #expect(cluster.frame.maxY <= contentView.bounds.maxY)
        #expect(cluster.frame.maxX <= contentView.bounds.maxX)
        #expect(contentView.bounds.maxY - cluster.frame.maxY == BrowserPanelControlCluster.margin)
        #expect(contentView.bounds.maxX - cluster.frame.maxX == BrowserPanelControlCluster.margin)
        // Buttons centred inside the capsule, with equal insets all round.
        for button in cluster.buttons {
            #expect(button.frame.minY == cluster.bounds.maxY - button.frame.maxY)
        }
        #expect(cluster.buttons.first?.frame.minX == cluster.bounds.maxX - (cluster.buttons.last?.frame.maxX ?? 0))

        let padding = cluster.convert(CGPoint(x: 1, y: cluster.bounds.midY), to: nil)
        let hit = try #require(contentView.hitTest(padding))
        #expect(hit === cluster || hit is BrowserPanelControlButton)
        #expect(!(hit is WKWebView))
    }

    @Test("the control capsule paints its own dark background so white symbols stay legible over any page")
    func controlClusterHasContrastingBackground() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "controls-contrast")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let contentView = try #require(coordinator.sessions[sessionID]?.windowController.panel.contentView)
        contentView.layoutSubtreeIfNeeded()

        let cluster = try #require(descendants(of: contentView).compactMap { $0 as? BrowserPanelControlCluster }.first)
        let background = try #require(cluster.layer?.backgroundColor)
        let color = try #require(NSColor(cgColor: background)?.usingColorSpace(.deviceRGB))
        // Opaque enough and dark enough to carry a white SF Symbol on a white page.
        #expect(color.alphaComponent >= 0.5)
        #expect(color.brightnessComponent <= 0.25)
        #expect(cluster.buttons.allSatisfy { $0.contentTintColor == .white })
        #expect(cluster.buttons.allSatisfy { ($0.toolTip?.isEmpty == false) })
        #expect(cluster.buttons.allSatisfy { $0.accessibilityLabel()?.isEmpty == false })
        #expect(cluster.isHidden == false)
    }

    @Test("clicking the capsule padding does not revoke the agent's Control Lease")
    func capsulePaddingClickDoesNotRevokeControlLease() async throws {
        _ = NSApplication.shared
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let sessionID = try await openSession(coordinator, connectionID: "capsule-lease")
        defer { coordinator.approveClose(sessionID: sessionID) }
        let session = try #require(coordinator.sessions[sessionID])
        let panel = session.windowController.panel
        let contentView = try #require(panel.contentView)
        contentView.layoutSubtreeIfNeeded()
        let cluster = try #require(descendants(of: contentView).compactMap { $0 as? BrowserPanelControlCluster }.first)

        var manualInputFired = false
        session.windowController.onManualInput = { manualInputFired = true }

        let padding = cluster.convert(CGPoint(x: 1, y: cluster.bounds.midY), to: nil)
        let event = try #require(NSEvent.mouseEvent(
            with: .leftMouseDown,
            location: padding,
            modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: panel.windowNumber,
            context: nil,
            eventNumber: 0,
            clickCount: 1,
            pressure: 1
        ))
        panel.sendEvent(event)

        #expect(manualInputFired == false)
    }

    private func descendants(of view: NSView) -> [NSView] {
        view.subviews + view.subviews.flatMap(descendants(of:))
    }

    private func openSession(_ coordinator: BrowserCoordinator, connectionID: String) async throws -> UUID {
        let existingSessionIDs = Set(coordinator.sessions.keys)
        let request = BrowserRPCRequest(
            id: .string(connectionID),
            method: "browser_open",
            params: [:]
        )
        _ = await coordinator.handle(connectionID: connectionID, connectionLabel: connectionID, request: request)
        return try #require(coordinator.sessions.keys.first(where: { !existingSessionIDs.contains($0) }))
    }
}
