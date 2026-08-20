import CoreGraphics

enum BrowserCueDragPhase: Equatable {
    case began
    case changed
    case ended
}

struct BrowserCueDragState: Equatable {
    enum Event: Equatable {
        case waiting
        case began
        case changed
        case clicked
        case ended
        case beganAndEnded
    }

    let cueBounds: CGRect
    let threshold: CGFloat

    private(set) var isDragging = false
    private var startLocation: CGPoint?

    init(cueBounds: CGRect = CGRect(x: 0, y: 0, width: 18, height: 18), threshold: CGFloat = 5) {
        self.cueBounds = cueBounds
        self.threshold = max(0, threshold)
    }

    mutating func update(startLocation: CGPoint, location: CGPoint) -> Event {
        if self.startLocation == nil {
            self.startLocation = startLocation
        }
        guard !isDragging else { return .changed }
        guard shouldBegin(at: location) else { return .waiting }
        isDragging = true
        return .began
    }

    mutating func end(startLocation: CGPoint, location: CGPoint) -> Event {
        if self.startLocation == nil {
            self.startLocation = startLocation
        }

        let event: Event
        if isDragging {
            event = .ended
        } else if shouldBegin(at: location) {
            // DragGesture normally delivers this movement through onChanged,
            // but keep the lifecycle complete if the final event is the first
            // one observed by the view.
            event = .beganAndEnded
        } else {
            event = .clicked
        }
        reset()
        return event
    }

    mutating func cancel() {
        reset()
    }

    private func shouldBegin(at location: CGPoint) -> Bool {
        guard let startLocation else { return false }
        let dx = location.x - startLocation.x
        let dy = location.y - startLocation.y
        let distance = hypot(dx, dy)
        return distance >= threshold || !cueBounds.contains(location)
    }

    private mutating func reset() {
        isDragging = false
        startLocation = nil
    }
}
