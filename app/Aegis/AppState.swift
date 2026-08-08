import Foundation
import Combine
import ServiceManagement
import AppKit

@MainActor
final class AppState: ObservableObject {
    private static let filterDesiredKey = "aegis.filterDesired"

    @Published var connected = false
    @Published var enabled = false
    @Published var filtering = false
    @Published var queries: UInt64 = 0
    @Published var blocked: UInt64 = 0
    @Published var domainCount: Int = 0
    @Published var listUpdatedAt: Int = 0
    @Published var lastError: String?
    @Published var version: String = "—"
    @Published var systemDNSActive = false
    @Published var dnsEffective = false
    @Published var dnsBypassReason: String?
    @Published var daemonServiceStatus: String = "unknown"
    @Published var statusLine: String = "Desconectado"
    @Published var listUpdating = false
    @Published var listUpdateMessage: String?

    let client = DaemonClient()
    let dnsManager = SystemDNSManager()
    let serviceManager = SMAppServiceManager()

    private var healthTask: Task<Void, Never>?

    /// User wants the filter on across launches (DNS is only applied when daemon is healthy).
    var filterDesired: Bool {
        get { UserDefaults.standard.bool(forKey: Self.filterDesiredKey) }
        set { UserDefaults.standard.set(newValue, forKey: Self.filterDesiredKey) }
    }

    var menuIconName: String {
        if dnsBypassReason != nil && filterActive { return "exclamationmark.shield" }
        if lastError != nil { return "exclamationmark.shield" }
        if filterActive && dnsEffective { return "checkmark.shield.fill" }
        return "shield"
    }

    /// Filter is only "on" when daemon is filtering AND system DNS points at Aegis.
    var filterActive: Bool {
        connected && enabled && filtering && systemDNSActive
    }

    var listUpdatedLabel: String {
        guard listUpdatedAt > 0 else { return "Nunca atualizada (ou desconhecida)" }
        let date = Date(timeIntervalSince1970: TimeInterval(listUpdatedAt))
        let rel = RelativeDateTimeFormatter()
        rel.locale = Locale(identifier: "pt_BR")
        rel.unitsStyle = .full
        let absolute = DateFormatter()
        absolute.locale = Locale(identifier: "pt_BR")
        absolute.dateStyle = .short
        absolute.timeStyle = .short
        return "\(rel.localizedString(for: date, relativeTo: Date())) · \(absolute.string(from: date))"
    }

    init() {
        Task { await bootstrap() }
    }

    /// First launch / reopen: never leave the Mac without DNS if aegisd is down.
    func bootstrap() async {
        failOpenIfDaemonDown(reason: "Daemon offline após iniciar — DNS restaurado (fail-open).")
        await refresh()
        startHealthMonitor()

        if filterDesired {
            if connected {
                do {
                    try await activateFilter()
                } catch {
                    lastError = error.localizedDescription
                }
            } else {
                // Try to (re)register LaunchDaemon, then activate if it comes up.
                try? serviceManager.registerIfNeeded()
                for _ in 0..<10 {
                    try? await Task.sleep(nanoseconds: 500_000_000)
                    if (try? await client.status()) != nil {
                        connected = true
                        break
                    }
                }
                if connected {
                    do {
                        try await activateFilter()
                    } catch {
                        lastError = error.localizedDescription
                    }
                } else {
                    failOpenIfDaemonDown(
                        reason: "Filtro desejado, mas o serviço não subiu. DNS restaurado — internet preservada. Registre o serviço em Ajustes → Avançado."
                    )
                    await refresh()
                }
            }
        }
    }

    func refresh() async {
        // No pre-flight ping: status() below is itself the reachability probe, and the catch
        // branch runs the same fail-open. The old extra ping was a blocking socket round-trip
        // on the main actor every 5 seconds.
        do {
            let status = try await client.status()
            connected = true
            domainCount = status.domainCount
            listUpdatedAt = status.listUpdatedAt
            version = status.version

            // Heal: user wants filter (or daemon still marked on) but DNS was released by fail-open.
            var healed = false
            if status.enabled && status.filtering && !dnsManager.isPointingToLocal() {
                healed = true
                if filterDesired {
                    do {
                        try dnsManager.activateLocalDNS()
                        _ = try await client.setEnabled(true)
                    } catch {
                        lastError = error.localizedDescription
                    }
                } else {
                    // Keep internet working; sync daemon flag to match reality.
                    _ = try? await client.setEnabled(false)
                }
            }

            // Only re-read when the branch above actually flipped the daemon's flags.
            let status2 = healed ? ((try? await client.status()) ?? status) : status
            enabled = status2.enabled
            filtering = status2.filtering

            if let err = status2.lastUpdateError, !err.isEmpty {
                lastError = err
            } else if lastError?.contains("DNS restaurado") != true {
                lastError = nil
            }

            let metrics = try await client.metrics()
            queries = metrics.queries
            blocked = metrics.blocked

            // Liveness is not health: the daemon can answer `status` perfectly while every
            // upstream is unreachable, and then the Mac points at 127.0.0.1 with no working
            // resolver. 25 failures in a row with nothing succeeding is not a blip.
            if metrics.consecutiveUpstreamFailures >= 25 && dnsManager.isPointingToLocal() {
                dnsManager.restoreDNS()
                _ = try? await client.setEnabled(false)
                systemDNSActive = false
                dnsEffective = false
                enabled = false
                filtering = false
                lastError = "O Aegis não conseguiu resolver nada nas últimas "
                    + "\(metrics.consecutiveUpstreamFailures) consultas. DNS restaurado para "
                    + "não deixar você sem internet. Verifique os servidores upstream em "
                    + "Ajustes → Avançado."
                statusLine = formatStatus()
                return
            }
            daemonServiceStatus = serviceManager.statusLabel()
        } catch {
            connected = false
            enabled = false
            filtering = false
            statusLine = "Daemon offline"
            if lastError == nil || lastError?.contains("DNS restaurado") != true {
                lastError = error.localizedDescription
            }
            daemonServiceStatus = serviceManager.statusLabel()
            failOpenIfDaemonDown(reason: "Daemon offline — DNS restaurado para não quebrar a internet.")
        }

        let probe = dnsManager.probeEffectiveDNS()
        systemDNSActive = probe.configuredLocal
        dnsEffective = probe.effectiveLocal
        dnsBypassReason = probe.bypassReason
        statusLine = formatStatus()
    }

    /// If Wi‑Fi/Ethernet still points at 127.0.0.1 but aegisd is dead, restore immediately.
    @discardableResult
    func failOpenIfDaemonDown(reason: String) -> Bool {
        let daemonUp = client.isReachableSync()
        guard !daemonUp else { return false }
        guard dnsManager.isPointingToLocal() else { return false }
        dnsManager.restoreDNS()
        lastError = reason
        systemDNSActive = false
        dnsEffective = false
        enabled = false
        filtering = false
        return true
    }

    /// Called on quit / reboot terminate — always release DNS so next boot is safe even if launchd fails.
    func prepareForTermination() {
        healthTask?.cancel()
        dnsManager.restoreDNS()
        // Do not wait on the daemon here (would risk deadlock on main during terminate).
    }

    func toggleEnabled(_ on: Bool) async {
        do {
            if on {
                filterDesired = true
                try await activateFilter()
            } else {
                filterDesired = false
                try await deactivateFilter()
            }
            await refresh()
        } catch {
            lastError = error.localizedDescription
            failOpenIfDaemonDown(reason: error.localizedDescription)
        }
    }

    func activateFilter() async throws {
        try serviceManager.registerIfNeeded()
        // Wait briefly for launchd to spawn after register
        for _ in 0..<8 {
            if client.isReachableSync() { break }
            try await Task.sleep(nanoseconds: 250_000_000)
        }
        guard client.isReachableSync() else {
            dnsManager.restoreDNS()
            throw AegisError.daemonUnreachable
        }
        _ = try await client.setEnabled(true)
        try dnsManager.activateLocalDNS()
        try await Task.sleep(nanoseconds: 400_000_000)
        await refresh()
        if !connected {
            dnsManager.restoreDNS()
            throw AegisError.daemonUnreachable
        }
        // Verify DNS works through daemon (fail-open if not)
        if !client.isReachableSync() {
            dnsManager.restoreDNS()
            throw AegisError.daemonUnreachable
        }
        if let reason = dnsBypassReason {
            lastError = reason
        }
        filterDesired = true
    }

    func deactivateFilter() async throws {
        filterDesired = false
        dnsManager.restoreDNS()
        _ = try? await client.setEnabled(false)
        await refresh()
    }

    func updateLists() async {
        guard !listUpdating else { return }
        listUpdating = true
        listUpdateMessage = "Baixando e compilando listas…"
        let before = listUpdatedAt
        let beforeCount = domainCount
        let beforeError = lastError
        defer { listUpdating = false }
        do {
            _ = try await client.updateLists()
            for i in 0..<90 {
                try await Task.sleep(nanoseconds: 1_000_000_000)
                await refresh()
                if listUpdatedAt != before || domainCount != beforeCount {
                    listUpdateMessage = "Pronto · \(Self.compact(UInt64(domainCount))) domínios · \(listUpdatedLabel)"
                    statusLine = formatStatus()
                    return
                }
                if i == 5 {
                    listUpdateMessage = "Ainda atualizando (listas grandes demoram)…"
                }
                // Any error the daemon raised since we started belongs to this update. Matching
                // English substrings missed every Portuguese message the updater now emits, so
                // a failed update just sat on "Baixando…" until the 90s timeout.
                if let err = lastError, err != beforeError {
                    listUpdateMessage = "Falhou: \(err)"
                    return
                }
            }
            listUpdateMessage = "Tempo esgotado — confira o status. Última: \(listUpdatedLabel)"
        } catch {
            listUpdateMessage = "Erro: \(error.localizedDescription)"
            lastError = error.localizedDescription
        }
    }

    func quitApp() async {
        filterDesired = false
        _ = try? await client.setEnabled(false)
        prepareForTermination()
        NSApplication.shared.terminate(nil)
    }

    private func startHealthMonitor() {
        healthTask?.cancel()
        healthTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 5_000_000_000)
                guard let self else { return }
                await self.refresh()
            }
        }
    }

    private func formatStatus() -> String {
        if !connected {
            return "Daemon offline"
        }
        if !systemDNSActive {
            return filterDesired ? "Ativando DNS…" : "Filtro off (DNS livre)"
        }
        if dnsBypassReason != nil {
            return "VPN/DNS contornando o Aegis"
        }
        let blockedFmt = Self.compact(blocked)
        let age: String
        if listUpdatedAt == 0 {
            age = "lista —"
        } else {
            let secs = Int(Date().timeIntervalSince1970) - listUpdatedAt
            if secs < 3600 {
                age = "lista \(max(secs / 60, 0))m"
            } else {
                age = "lista \(secs / 3600)h"
            }
        }
        return "\(blockedFmt) bloqueados · \(age)"
    }

    static func compact(_ n: UInt64) -> String {
        if n >= 1_000_000 { return String(format: "%.1fM", Double(n) / 1_000_000) }
        if n >= 1_000 { return String(format: "%.1fk", Double(n) / 1_000) }
        return "\(n)"
    }
}

enum AegisError: LocalizedError {
    case daemonUnreachable
    case dnsFailed(String)

    var errorDescription: String? {
        switch self {
        case .daemonUnreachable:
            return "Daemon não responde — DNS não foi alterado (fail-open)."
        case .dnsFailed(let s):
            return s
        }
    }
}
