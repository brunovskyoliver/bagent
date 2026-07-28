import Foundation

/// Separates BaseRT transport chunks from the cadence shown in the notch.
/// Canonical text remains owned by ChatMessage; this module only emits display edits.
@MainActor
final class AdaptiveStreamPresenter {
    typealias DisplayEdit = @MainActor (String) -> Void

    private var buffer = ""
    private var worker: Task<Void, Never>?
    private var finishing = false
    private var finishDeadline: Date?
    private let emit: DisplayEdit

    init(emit: @escaping DisplayEdit) {
        self.emit = emit
    }

    func enqueue(_ text: String) {
        guard !text.isEmpty else { return }
        buffer += text
        startIfNeeded()
    }

    func finish() async {
        finishing = true
        finishDeadline = Date().addingTimeInterval(0.45)
        startIfNeeded()
        await worker?.value
    }

    func cancel() {
        worker?.cancel()
        worker = nil
        buffer = ""
    }

    private func startIfNeeded() {
        guard worker == nil, !buffer.isEmpty else { return }
        worker = Task { @MainActor [weak self] in
            guard let self else { return }
            while !Task.isCancelled, !buffer.isEmpty {
                if let finishDeadline, Date() >= finishDeadline {
                    emit(buffer)
                    buffer = ""
                    break
                }
                guard let edit = takeDisplayEdit() else {
                    try? await Task.sleep(nanoseconds: 12_000_000)
                    continue
                }
                emit(edit)
                let backlog = buffer.count
                let delay: UInt64
                if finishing {
                    delay = backlog > 80 ? 6_000_000 : 14_000_000
                } else if backlog > 240 {
                    delay = 10_000_000
                } else if backlog > 80 {
                    delay = 20_000_000
                } else {
                    delay = edit.contains(where: { ".!?\n".contains($0) })
                        ? 58_000_000 : 34_000_000
                }
                try? await Task.sleep(nanoseconds: delay)
            }
            worker = nil
        }
    }

    /// Emits one lexical word with adjacent whitespace. If the provider split a
    /// word, wait for more data unless the stream is finishing.
    private func takeDisplayEdit() -> String? {
        guard !buffer.isEmpty else { return nil }
        var sawNonWhitespace = false
        var end = buffer.startIndex
        var cursor = buffer.startIndex
        while cursor < buffer.endIndex {
            let character = buffer[cursor]
            let next = buffer.index(after: cursor)
            if character.isWhitespace {
                if sawNonWhitespace {
                    end = next
                    while end < buffer.endIndex, buffer[end].isWhitespace {
                        end = buffer.index(after: end)
                    }
                    break
                }
            } else {
                sawNonWhitespace = true
                end = next
            }
            cursor = next
        }

        if end == buffer.endIndex, !finishing,
           let last = buffer.last, !last.isWhitespace,
           !".,;:!?)]}".contains(last) {
            // The provider may have split a word; wait for its next delta.
            return nil
        }
        let edit = String(buffer[..<end])
        buffer.removeSubrange(..<end)
        return edit
    }
}
