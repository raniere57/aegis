import Foundation

struct DaemonStatus: Decodable {
    let enabled: Bool
    let filtering: Bool
    let uptimeSecs: UInt64
    let domainCount: Int
    let listUpdatedAt: Int
    let lastUpdateError: String?
    let version: String

    enum CodingKeys: String, CodingKey {
        case enabled, filtering, version
        case uptimeSecs = "uptime_secs"
        case domainCount = "domain_count"
        case listUpdatedAt = "list_updated_at"
        case lastUpdateError = "last_update_error"
    }
}

struct DaemonMetrics: Decodable {
    let queries: UInt64
    let blocked: UInt64
    let cacheHit: UInt64
    let cacheMiss: UInt64
    let upstreamOk: UInt64
    let upstreamErrors: UInt64
    let consecutiveUpstreamFailures: UInt64

    enum CodingKeys: String, CodingKey {
        case queries, blocked
        case cacheHit = "cache_hit"
        case cacheMiss = "cache_miss"
        case upstreamOk = "upstream_ok"
        case upstreamErrors = "upstream_errors"
        case consecutiveUpstreamFailures = "consecutive_upstream_failures"
    }
}

struct RPCResponse<T: Decodable>: Decodable {
    let id: String
    let ok: Bool
    let result: T?
    let error: RPCError?
}

struct RPCError: Decodable {
    let code: String
    let message: String
}

struct ListSourceStat {
    let url: String
    let domainCount: Int
    let lastSuccessUnix: Int?
    let lastError: String?
}

struct RecentBlock: Identifiable {
    let domain: String
    let atUnix: Int
    let hits: Int
    var id: String { domain }
}

struct ListsInfo {
    let urls: [String]
    let autoUpdate: Bool
    let intervalHours: Int
    let listCount: Int
    let uniqueDomains: Int
    let sumDomainCounts: Int
    let sources: [ListSourceStat]
}

final class DaemonClient: @unchecked Sendable {
    /// Last socket that answered successfully.
    private(set) var activeSocketPath: String?
    private let lock = NSLock()

    static var candidateSockets: [String] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        // Prefer privileged socket first when both exist — launchd uses /var/run.
        return [
            "/var/run/aegis.sock",
            "\(home)/.aegis/aegis.sock",
        ]
    }

    func ping() async throws -> String {
        let raw: [String: Any] = try await call(method: "ping", params: [:])
        return raw["version"] as? String ?? "?"
    }

    func status() async throws -> DaemonStatus {
        try await callDecodable(method: "status", params: [:])
    }

    func metrics() async throws -> DaemonMetrics {
        try await callDecodable(method: "metrics", params: [:])
    }

    func setEnabled(_ enabled: Bool) async throws -> Bool {
        let raw: [String: Any] = try await call(method: "set_enabled", params: ["enabled": enabled])
        return raw["enabled"] as? Bool ?? enabled
    }

    @discardableResult
    func updateLists() async throws -> Bool {
        let raw: [String: Any] = try await call(method: "update_lists", params: [:])
        return raw["started"] as? Bool ?? false
    }

    func getConfig() async throws -> [String: Any] {
        try await call(method: "get_config", params: [:])
    }

    func patchConfig(_ params: [String: Any]) async throws {
        let _: [String: Any] = try await call(method: "patch_config", params: params)
    }

    func allowlistList() async throws -> [String] {
        let raw: [String: Any] = try await call(method: "allowlist.list", params: [:])
        return raw["domains"] as? [String] ?? []
    }

    func allowlistAdd(_ domain: String) async throws -> [String] {
        let raw: [String: Any] = try await call(method: "allowlist.add", params: ["domain": domain])
        return raw["domains"] as? [String] ?? []
    }

    func allowlistRemove(_ domain: String) async throws -> [String] {
        let raw: [String: Any] = try await call(method: "allowlist.remove", params: ["domain": domain])
        return raw["domains"] as? [String] ?? []
    }

    func listsList() async throws -> ListsInfo {
        let raw: [String: Any] = try await call(method: "lists.list", params: [:])
        let urls = raw["urls"] as? [String] ?? []
        let auto = raw["auto_update"] as? Bool ?? true
        let hours = raw["interval_hours"] as? Int ?? 24
        let listCount = raw["list_count"] as? Int ?? urls.count
        let unique = raw["unique_domains"] as? Int ?? 0
        let sum = raw["sum_domain_counts"] as? Int ?? 0
        var sources: [ListSourceStat] = []
        if let arr = raw["sources"] as? [[String: Any]] {
            sources = arr.map { row in
                ListSourceStat(
                    url: row["url"] as? String ?? "",
                    domainCount: row["domain_count"] as? Int ?? 0,
                    lastSuccessUnix: row["last_success_unix"] as? Int,
                    lastError: row["last_error"] as? String
                )
            }
        }
        return ListsInfo(
            urls: urls,
            autoUpdate: auto,
            intervalHours: hours,
            listCount: listCount,
            uniqueDomains: unique,
            sumDomainCounts: sum,
            sources: sources
        )
    }

    func recentBlocked(limit: Int = 50) async throws -> [RecentBlock] {
        let raw: [String: Any] = try await call(method: "recent.blocked", params: ["limit": limit])
        let rows = raw["entries"] as? [[String: Any]] ?? []
        return rows.compactMap { row in
            guard let domain = row["domain"] as? String else { return nil }
            return RecentBlock(
                domain: domain,
                atUnix: row["at_unix"] as? Int ?? 0,
                hits: row["hits"] as? Int ?? 1
            )
        }
    }

    @discardableResult
    func recentClear() async throws -> Bool {
        let raw: [String: Any] = try await call(method: "recent.clear", params: [:])
        return raw["cleared"] as? Bool ?? true
    }

    func listsAdd(_ url: String) async throws {
        let _: [String: Any] = try await call(method: "lists.add_url", params: ["url": url])
    }

    func listsRemove(_ url: String) async throws {
        let _: [String: Any] = try await call(method: "lists.remove_url", params: ["url": url])
    }

    /// Fast synchronous reachability check for fail-open paths (not on main UI thread ideally).
    func isReachableSync() -> Bool {
        (try? syncRPCTryingCandidates(method: "ping", params: [:])) != nil
    }

    private func callDecodable<T: Decodable>(method: String, params: [String: Any]) async throws -> T {
        let data = try await callRaw(method: method, params: params)
        let decoded = try JSONDecoder().decode(RPCResponse<T>.self, from: data)
        if decoded.ok, let result = decoded.result {
            return result
        }
        throw NSError(
            domain: "Aegis",
            code: 1,
            userInfo: [NSLocalizedDescriptionKey: decoded.error?.message ?? "erro desconhecido"]
        )
    }

    private func call(method: String, params: [String: Any]) async throws -> [String: Any] {
        let data = try await callRaw(method: method, params: params)
        let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
        guard (obj["ok"] as? Bool) == true else {
            let err = obj["error"] as? [String: Any]
            let msg = err?["message"] as? String ?? "erro"
            throw NSError(domain: "Aegis", code: 1, userInfo: [NSLocalizedDescriptionKey: msg])
        }
        return obj["result"] as? [String: Any] ?? [:]
    }

    private func callRaw(method: String, params: [String: Any]) async throws -> Data {
        try await withCheckedThrowingContinuation { cont in
            DispatchQueue.global(qos: .userInitiated).async {
                do {
                    let data = try self.syncRPCTryingCandidates(method: method, params: params)
                    cont.resume(returning: data)
                } catch {
                    cont.resume(throwing: error)
                }
            }
        }
    }

    private func syncRPCTryingCandidates(method: String, params: [String: Any]) throws -> Data {
        lock.lock()
        let preferred = activeSocketPath
        lock.unlock()

        var paths = Self.candidateSockets
        if let preferred {
            paths.removeAll { $0 == preferred }
            paths.insert(preferred, at: 0)
        }

        var lastError: Error = POSIXError(.ECONNREFUSED)
        for path in paths {
            // Skip missing paths quickly
            if !FileManager.default.fileExists(atPath: path) { continue }
            do {
                let data = try Self.syncRPC(socketPath: path, method: method, params: params)
                lock.lock()
                activeSocketPath = path
                lock.unlock()
                // Stale user socket leftover from --dev? remove if we connected to privileged
                if path == "/var/run/aegis.sock" {
                    let homeSock = FileManager.default.homeDirectoryForCurrentUser
                        .appendingPathComponent(".aegis/aegis.sock").path
                    try? FileManager.default.removeItem(atPath: homeSock)
                }
                return data
            } catch {
                lastError = error
                // Stale socket file with no listener — remove user-level leftover
                if path.contains(".aegis/aegis.sock") {
                    try? FileManager.default.removeItem(atPath: path)
                }
            }
        }
        throw lastError
    }

    private static func syncRPC(socketPath: String, method: String, params: [String: Any]) throws -> Data {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw POSIXError(.ECONNREFUSED) }
        defer { close(fd) }

        // Without these the read below blocks forever if the daemon accepts and then stalls
        // (it holds no lock across a list compile today, but a wedged daemon must not wedge the
        // UI too — every status tick would pile up on a global queue thread).
        var tv = timeval(tv_sec: 15, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = socketPath.utf8CString
        guard pathBytes.count <= MemoryLayout.size(ofValue: addr.sun_path) else {
            throw POSIXError(.ENAMETOOLONG)
        }
        withUnsafeMutablePointer(to: &addr.sun_path) { ptr in
            ptr.withMemoryRebound(to: CChar.self, capacity: pathBytes.count) { dest in
                for (i, b) in pathBytes.enumerated() {
                    dest[i] = b
                }
            }
        }

        let len = socklen_t(MemoryLayout<sockaddr_un>.size)
        let connectResult = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                Darwin.connect(fd, sockPtr, len)
            }
        }
        guard connectResult == 0 else { throw POSIXError(.ECONNREFUSED) }

        let id = UUID().uuidString
        let payload: [String: Any] = ["id": id, "method": method, "params": params]
        var body = try JSONSerialization.data(withJSONObject: payload)
        body.append(contentsOf: [0x0A])
        let written = body.withUnsafeBytes { Darwin.write(fd, $0.baseAddress, body.count) }
        guard written == body.count else { throw POSIXError(.EIO) }

        var buffer = [UInt8](repeating: 0, count: 65536)
        var collected = Data()
        while true {
            let n = Darwin.read(fd, &buffer, buffer.count)
            if n <= 0 { break }
            collected.append(contentsOf: buffer[0..<n])
            if collected.contains(0x0A) { break }
        }
        // No newline means the peer closed (or timed out) mid-response; surfacing that as an
        // error is what lets the caller fall through to the next candidate socket instead of
        // failing on a JSON parse of a truncated object.
        guard let idx = collected.firstIndex(of: 0x0A) else { throw POSIXError(.ETIMEDOUT) }
        return Data(collected.prefix(upTo: idx))
    }
}
