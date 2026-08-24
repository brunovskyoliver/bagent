import Foundation

struct BrowserNavigationPolicy: Sendable {
    typealias Resolver = @Sendable (String) -> [String]

    let resolver: Resolver

    init(resolver: @escaping Resolver = BrowserNavigationPolicy.resolveHost) {
        self.resolver = resolver
    }

    func validate(_ url: URL) -> Result<Void, BrowserFailure> {
        guard let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https",
              let host = url.host, !host.isEmpty else {
            return .failure(BrowserFailure(.navigationBlocked, "Only http and https top-level navigation is allowed."))
        }

        let addresses = isIPAddress(host) ? [host] : resolver(host)
        guard !addresses.isEmpty else {
            return .failure(BrowserFailure(.navigationBlocked, "The destination did not resolve to an allowlisted address."))
        }

        let allowed = addresses.filter(Self.isAllowedAddress)
        guard allowed.count == addresses.count else {
            if !allowed.isEmpty {
                return .failure(BrowserFailure(.mixedDNSAnswer, "Every A and AAAA answer must be inside the Navigation Allowlist."))
            }
            return .failure(BrowserFailure(.navigationBlocked, "The destination is outside the Navigation Allowlist."))
        }
        return .success(())
    }

    func origin(for url: URL) -> String? {
        guard let scheme = url.scheme?.lowercased(), let host = url.host else { return nil }
        let port = url.port.map { ":\($0)" } ?? ""
        return "\(scheme)://\(host)\(port)"
    }

    private func isIPAddress(_ host: String) -> Bool {
        var v4 = in_addr()
        var v6 = in6_addr()
        return host.withCString { inet_pton(AF_INET, $0, &v4) == 1 }
            || host.withCString { inet_pton(AF_INET6, $0, &v6) == 1 }
    }

    private static func isAllowedAddress(_ address: String) -> Bool {
        var v4 = in_addr()
        if address.withCString({ inet_pton(AF_INET, $0, &v4) }) == 1 {
            let value = UInt32(bigEndian: v4.s_addr)
            let first = value >> 24
            let second = (value >> 16) & 0xff
            return first == 127
                || first == 172 && (second == 19 || second == 29)
        }

        var v6 = in6_addr()
        guard address.withCString({ inet_pton(AF_INET6, $0, &v6) }) == 1 else { return false }
        let bytes = withUnsafeBytes(of: v6) { Array($0) }
        return bytes == Array(repeating: UInt8(0), count: 15) + [1]
    }

    private static func resolveHost(_ host: String) -> [String] {
        var hints = addrinfo(
            ai_flags: 0,
            ai_family: AF_UNSPEC,
            ai_socktype: SOCK_STREAM,
            ai_protocol: 0,
            ai_addrlen: 0,
            ai_canonname: nil,
            ai_addr: nil,
            ai_next: nil
        )
        var result: UnsafeMutablePointer<addrinfo>?
        guard getaddrinfo(host, nil, &hints, &result) == 0, let first = result else { return [] }
        defer { freeaddrinfo(first) }

        var addresses: [String] = []
        var cursor: UnsafeMutablePointer<addrinfo>? = first
        while let info = cursor {
            let family = info.pointee.ai_family
            var buffer = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(info.pointee.ai_addr, info.pointee.ai_addrlen, &buffer, socklen_t(buffer.count), nil, 0, NI_NUMERICHOST) == 0 {
                let bytes = buffer.prefix(while: { $0 != 0 }).map { UInt8(bitPattern: $0) }
                addresses.append(String(decoding: bytes, as: UTF8.self))
            }
            _ = family
            cursor = info.pointee.ai_next
        }
        return Array(Set(addresses))
    }
}
