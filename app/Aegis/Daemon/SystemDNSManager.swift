import Foundation
import AppKit

/// Backs up and restores per-service DNS via `networksetup`.
final class SystemDNSManager {
    private let backupURL: URL

    struct EffectiveDNS {
        /// Wi‑Fi/Ethernet configured to 127.0.0.1
        var configuredLocal: Bool
        /// Primary resolver from scutil is actually 127.0.0.1
        var effectiveLocal: Bool
        /// Human-readable reason when traffic bypasses Aegis
        var bypassReason: String?
        var primaryServers: [String]
    }

    init() {
        let base = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Aegis", isDirectory: true)
        try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        backupURL = base.appendingPathComponent("dns-backup.json")
    }

    /// The set of network services changes only when hardware is plugged in or a VPN adapter is
    /// installed, but this was re-spawning `networksetup` on every 5s status tick. Cache it.
    private var servicesCache: (at: Date, names: [String])?
    private static let servicesCacheTTL: TimeInterval = 60

    func networkServices() -> [String] {
        if let c = servicesCache, Date().timeIntervalSince(c.at) < Self.servicesCacheTTL {
            return c.names
        }
        let names = fetchNetworkServices()
        servicesCache = (Date(), names)
        return names
    }

    private func fetchNetworkServices() -> [String] {
        let out = run("/usr/sbin/networksetup", ["-listallnetworkservices"]) ?? ""
        return out
            .split(separator: "\n")
            .map(String.init)
            .filter { !$0.isEmpty && !$0.hasPrefix("*") && !$0.contains("An asterisk") }
    }

    func currentDNS(for service: String) -> [String] {
        let out = run("/usr/sbin/networksetup", ["-getdnsservers", service]) ?? ""
        if out.lowercased().contains("there aren't any") || out.lowercased().contains("aren't any dns") {
            return []
        }
        return out
            .split(separator: "\n")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    func isPointingToLocal() -> Bool {
        for svc in networkServices() {
            let dns = currentDNS(for: svc)
            if dns.contains("127.0.0.1") || dns.contains("::1") {
                return true
            }
        }
        return false
    }

    /// What the OS actually uses (VPN can override Wi‑Fi DNS).
    func probeEffectiveDNS() -> EffectiveDNS {
        let configured = isPointingToLocal()
        let scutil = run("/usr/sbin/scutil", ["--dns"]) ?? ""
        let primary = parsePrimaryNameservers(scutil)
        let effectiveLocal = primary.contains("127.0.0.1") || primary.contains("::1")

        var reason: String?
        if configured && !effectiveLocal {
            let lower = scutil.lowercased()
            if lower.contains("ppp0") || lower.contains("utun") || lower.contains("ipsec") {
                reason = "VPN ativa impondo outro DNS (\(primary.joined(separator: ", "))). O Aegis está configurado no Wi‑Fi, mas o sistema não o usa. Desative a VPN ou o “DNS da VPN”."
            } else {
                reason = "O DNS efetivo do Mac é \(primary.joined(separator: ", ")), não 127.0.0.1. Algo está contornando o Aegis (VPN, perfil, DoH)."
            }
        } else if !configured {
            // Soft inactive — not a VPN bypass; UI decides how to present.
            reason = nil
        }

        return EffectiveDNS(
            configuredLocal: configured,
            effectiveLocal: effectiveLocal,
            bypassReason: reason,
            primaryServers: primary
        )
    }

    /// Loopback addresses we install. Never let these into the backup: after a crash the
    /// system DNS is already 127.0.0.1, and snapshotting that would make restoreDNS() *set*
    /// loopback forever — inverting fail-open so every safety net kills the internet instead
    /// of saving it. An all-empty snapshot restores to "Empty" (DHCP), which is correct both
    /// here and on an ordinary DHCP Mac.
    private static let loopbackServers: Set<String> = ["127.0.0.1", "::1"]

    func activateLocalDNS() throws {
        // Writing DNS must cover an interface plugged in seconds ago, so never trust the cache.
        servicesCache = nil
        var services: [[String: Any]] = []
        for name in networkServices() {
            let servers = currentDNS(for: name).filter { !Self.loopbackServers.contains($0) }
            services.append(["name": name, "servers": servers])
        }
        let payload: [String: Any] = [
            "saved_at": ISO8601DateFormatter().string(from: Date()),
            "services": services,
        ]
        let data = try JSONSerialization.data(withJSONObject: payload, options: [.prettyPrinted])
        try data.write(to: backupURL, options: .atomic)

        for name in networkServices() {
            _ = run("/usr/sbin/networksetup", ["-setdnsservers", name, "127.0.0.1", "::1"])
        }
        flushDNSCache()
    }

    func restoreDNS() {
        servicesCache = nil
        guard let data = try? Data(contentsOf: backupURL),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let services = obj["services"] as? [[String: Any]]
        else {
            for name in networkServices() {
                _ = run("/usr/sbin/networksetup", ["-setdnsservers", name, "Empty"])
            }
            flushDNSCache()
            return
        }

        for svc in services {
            guard let name = svc["name"] as? String else { continue }
            let servers = svc["servers"] as? [String] ?? []
            if servers.isEmpty {
                _ = run("/usr/sbin/networksetup", ["-setdnsservers", name, "Empty"])
            } else {
                var args = ["-setdnsservers", name]
                args.append(contentsOf: servers)
                _ = run("/usr/sbin/networksetup", args)
            }
        }
        flushDNSCache()
    }

    private func parsePrimaryNameservers(_ scutil: String) -> [String] {
        // First resolver block's nameserver lines
        var servers: [String] = []
        var inResolver = false
        for line in scutil.split(separator: "\n", omittingEmptySubsequences: false) {
            let t = line.trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("resolver #") {
                if inResolver { break }
                inResolver = true
                continue
            }
            if inResolver, t.hasPrefix("nameserver[") {
                if let colon = t.range(of: ":") {
                    let ip = t[colon.upperBound...].trimmingCharacters(in: .whitespaces)
                    if !ip.isEmpty { servers.append(ip) }
                }
            }
        }
        return servers
    }

    private func flushDNSCache() {
        _ = run("/usr/bin/dscacheutil", ["-flushcache"])
        _ = run("/usr/bin/killall", ["-HUP", "mDNSResponder"])
    }

    /// Returns stdout+stderr, or nil if the process could not be launched or exited non-zero.
    /// The autoreleasepool + explicit close matter: this runs several times per 5s status tick,
    /// and without them each call leaks roughly 3.4 KB of Foundation objects and one file
    /// descriptor pair, which is what grew the app to hundreds of MB over a day.
    @discardableResult
    private func run(_ launchPath: String, _ args: [String]) -> String? {
        autoreleasepool {
            let task = Process()
            task.executableURL = URL(fileURLWithPath: launchPath)
            task.arguments = args
            let pipe = Pipe()
            task.standardOutput = pipe
            task.standardError = pipe
            do {
                try task.run()
                // Read before waiting: a child that fills the 64 KB pipe buffer blocks forever
                // if we wait for exit first. networksetup output is small today, but this is
                // the kind of deadlock that only shows up on the machine with 30 interfaces.
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                task.waitUntilExit()
                try? pipe.fileHandleForReading.close()
                try? pipe.fileHandleForWriting.close()
                guard task.terminationStatus == 0 else { return nil }
                return String(data: data, encoding: .utf8)
            } catch {
                return nil
            }
        }
    }
}
