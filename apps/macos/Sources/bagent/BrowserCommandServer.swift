import Darwin
import Foundation

private func browserPeerBelongsToCurrentUser(_ fd: Int32) -> Bool {
    var effectiveUID: uid_t = 0
    var effectiveGID: gid_t = 0
    guard getpeereid(fd, &effectiveUID, &effectiveGID) == 0 else { return false }
    return effectiveUID == geteuid()
}

private final class BrowserSocketClient: @unchecked Sendable {
    let fileDescriptor: Int32
    let connectionID: String
    var buffer = Data()
    var source: DispatchSourceRead?

    init(fileDescriptor: Int32, connectionID: String) {
        self.fileDescriptor = fileDescriptor
        self.connectionID = connectionID
    }
}

@MainActor
final class BrowserCommandServer {
    static var socketURL: URL {
        if let override = ProcessInfo.processInfo.environment["BAGENT_BROWSER_SOCKET"] {
            return URL(fileURLWithPath: override)
        }
        let applicationSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        return applicationSupport.appendingPathComponent("bagent", isDirectory: true).appendingPathComponent("browser.sock")
    }

    private let coordinator: BrowserCoordinator
    private var listenerFileDescriptor: Int32 = -1
    private var listenerSource: DispatchSourceRead?
    private var clients: [String: BrowserSocketClient] = [:]

    init(coordinator: BrowserCoordinator) {
        self.coordinator = coordinator
    }

    func start() {
        guard listenerFileDescriptor == -1 else { return }
        do {
            let path = Self.socketURL.path
            let directory = Self.socketURL.deletingLastPathComponent()
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            chmod(directory.path, mode_t(0o700))

            if let existing = existingListener(path: path) {
                close(existing)
                return
            }
            unlink(path)

            let fd = socket(AF_UNIX, SOCK_STREAM, 0)
            guard fd >= 0 else { return }
            var address = sockaddr_un()
            address.sun_family = sa_family_t(AF_UNIX)
            let pathBytes = Array(path.utf8) + [0]
            guard pathBytes.count <= MemoryLayout.size(ofValue: address.sun_path) else { return }
            withUnsafeMutableBytes(of: &address.sun_path) { buffer in
                buffer.copyBytes(from: pathBytes)
            }
            let addressLength = socklen_t(MemoryLayout<sa_family_t>.size + pathBytes.count)
            let bindResult = withUnsafePointer(to: &address) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    bind(fd, $0, addressLength)
                }
            }
            guard bindResult == 0, listen(fd, 8) == 0 else {
                close(fd)
                return
            }
            guard fcntl(fd, F_SETFL, O_NONBLOCK) == 0 else {
                close(fd)
                return
            }
            chmod(path, mode_t(0o600))
            listenerFileDescriptor = fd

            // The listener and client sources run on the main queue. This keeps
            // socket callbacks in the same isolation domain as the coordinator
            // and all WebKit/AppKit state, while the reads remain non-blocking.
            let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: .main)
            source.setEventHandler { [weak self] in
                while true {
                    var peerAddress = sockaddr_storage()
                    var peerLength = socklen_t(MemoryLayout<sockaddr_storage>.size)
                    let clientFD = withUnsafeMutablePointer(to: &peerAddress) { pointer in
                        pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                            accept(fd, $0, &peerLength)
                        }
                    }
                    guard clientFD >= 0 else { break }
                    // macOS may propagate the listener's nonblocking flag to
                    // accepted sockets. Reads are driven by DispatchSource;
                    // response writes must be allowed to drain completely.
                    _ = fcntl(clientFD, F_SETFL, 0)
                    if !browserPeerBelongsToCurrentUser(clientFD) {
                        close(clientFD)
                        continue
                    }
                    let connectionID = UUID().uuidString
                    self?.startClient(fileDescriptor: clientFD, connectionID: connectionID)
                }
            }
            source.setCancelHandler { close(fd) }
            source.resume()
            listenerSource = source
        } catch {
            listenerFileDescriptor = -1
        }
    }

    func stop() {
        listenerSource?.cancel()
        listenerSource = nil
        listenerFileDescriptor = -1
        for client in clients.values {
            client.source?.cancel()
            coordinator.connectionDidClose(client.connectionID)
        }
        clients.removeAll()
        unlink(Self.socketURL.path)
    }

    private func startClient(fileDescriptor: Int32, connectionID: String) {
        let client = BrowserSocketClient(fileDescriptor: fileDescriptor, connectionID: connectionID)
        clients[connectionID] = client
        let source = DispatchSource.makeReadSource(fileDescriptor: fileDescriptor, queue: .main)
        source.setEventHandler { [weak self, weak client] in
            guard let client else { return }
            var bytes = [UInt8](repeating: 0, count: 64 * 1024)
            let count = recv(fileDescriptor, &bytes, bytes.count, 0)
            if count < 0 && (errno == EAGAIN || errno == EWOULDBLOCK) {
                return
            }
            guard count > 0 else {
                self?.finishClient(client)
                return
            }
            client.buffer.append(contentsOf: bytes.prefix(count))
            while let frame = Self.nextFrame(from: &client.buffer) {
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    let response = await self.handle(frame: frame, client: client)
                    Self.write(response, to: client.fileDescriptor)
                }
            }
        }
        source.setCancelHandler { close(fileDescriptor) }
        client.source = source
        source.resume()
    }

    private func finishClient(_ client: BrowserSocketClient) {
        client.source?.cancel()
        clients.removeValue(forKey: client.connectionID)
        coordinator.connectionDidClose(client.connectionID)
    }

    private func handle(frame: Data, client: BrowserSocketClient) async -> Data {
        let request: BrowserRPCRequest
        do {
            request = try JSONDecoder().decode(BrowserRPCRequest.self, from: frame)
        } catch {
            let failure = BrowserRPCResponse.failure(id: .null, BrowserFailure(.invalidRequest, "Invalid browser RPC request."))
            return (try? JSONEncoder().encode(failure)) ?? Data()
        }
        let response = await coordinator.handle(connectionID: client.connectionID, connectionLabel: "stdio-\(client.connectionID.prefix(8))", request: request)
        return (try? JSONEncoder().encode(response)) ?? Data()
    }

    private func existingListener(path: String) -> Int32? {
        guard FileManager.default.fileExists(atPath: path) else { return nil }
        let probe = socket(AF_UNIX, SOCK_STREAM, 0)
        guard probe >= 0 else { return nil }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = Array(path.utf8) + [0]
        guard pathBytes.count <= MemoryLayout.size(ofValue: address.sun_path) else {
            close(probe)
            return nil
        }
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: pathBytes) }
        let length = socklen_t(MemoryLayout<sa_family_t>.size + pathBytes.count)
        let result = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(probe, $0, length) }
        }
        if result == 0 { return probe }
        close(probe)
        return nil
    }

    private static func nextFrame(from buffer: inout Data) -> Data? {
        guard buffer.count >= 4 else { return nil }
        let header = Array(buffer.prefix(4))
        let length = (UInt32(header[0]) << 24)
            | (UInt32(header[1]) << 16)
            | (UInt32(header[2]) << 8)
            | UInt32(header[3])
        guard length <= 8 * 1024 * 1024 else {
            buffer.removeAll()
            return nil
        }
        guard buffer.count >= 4 + Int(length) else { return nil }
        let frame = Data(buffer.dropFirst(4).prefix(Int(length)))
        buffer.removeFirst(4 + Int(length))
        return frame
    }

    nonisolated private static func write(_ response: Data, to fd: Int32) {
        var length = UInt32(response.count).bigEndian
        var frame = Data(bytes: &length, count: MemoryLayout<UInt32>.size)
        frame.append(response)
        frame.withUnsafeBytes { raw in
            var sent = 0
            while sent < raw.count {
                let count = send(fd, raw.baseAddress!.advanced(by: sent), raw.count - sent, 0)
                if count <= 0 { break }
                sent += count
            }
        }
    }
}
