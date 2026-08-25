import Foundation
import ServiceManagement
import AppKit

/// Registers the bundled LaunchDaemon + fail-open LaunchAgent via SMAppService (macOS 13+).
final class SMAppServiceManager {
    static let daemonPlistName = "com.aegis.daemon"
    static let failOpenPlistName = "com.aegis.failopen"

    func statusLabel() -> String {
        if #available(macOS 13.0, *) {
            let service = SMAppService.daemon(plistName: "\(Self.daemonPlistName).plist")
            switch service.status {
            case .enabled: return "ativo"
            case .requiresApproval: return "aguardando aprovação"
            case .notRegistered: return "não registrado"
            case .notFound: return "não encontrado"
            @unknown default: return "desconhecido"
            }
        }
        return "indisponível"
    }

    func registerIfNeeded() throws {
        let classic = "/Library/LaunchDaemons/com.aegis.daemon.plist"
        if FileManager.default.fileExists(atPath: classic) {
            // Already on the reliable path — don't fight it with SMAppService. Standing aside is
            // not enough, though: if an SMAppService registration still owns the label, launchd
            // keeps honoring IT, and it points at the executable inside the app bundle. The
            // classic plist then sits on disk being ignored while a months-old binary serves
            // DNS, and every installer that only checks "is a pid running" reports success.
            if #available(macOS 13.0, *) {
                let daemon = SMAppService.daemon(plistName: "\(Self.daemonPlistName).plist")
                if daemon.status == .enabled || daemon.status == .requiresApproval {
                    try? daemon.unregister()
                }
                try? registerFailOpenAgent()
            }
            return
        }
        if #available(macOS 13.0, *) {
            if needsClassicRepair() {
                try installClassicLaunchDaemon()
                try? registerFailOpenAgent()
                return
            }
            try registerDaemon()
            try registerFailOpenAgent()
        } else {
            try installClassicLaunchDaemon()
        }
    }

    private func needsClassicRepair() -> Bool {
        if #available(macOS 13.0, *) {
            let service = SMAppService.daemon(plistName: "\(Self.daemonPlistName).plist")
            // SM thinks it's registered/enabled but classic plist isn't there → often EX_CONFIG.
            return service.status == .enabled || service.status == .requiresApproval
        }
        return true
    }

    /// Force re-register after binary/codesign changes (fixes launchd EX_CONFIG / LWCR mismatch).
    /// Prefers classic /Library/LaunchDaemons with absolute path (reliable across reboot).
    func repairRegistration() throws {
        try installClassicLaunchDaemon()
        if #available(macOS 13.0, *) {
            try? registerFailOpenAgent()
        }
    }

    /// Installs `/Library/LaunchDaemons/com.aegis.daemon.plist` via admin osascript.
    func installClassicLaunchDaemon() throws {
        // Only ever run the script shipped inside our own bundle — this executes as root
        // via osascript, so an out-of-bundle path would be an arbitrary-code-execution hole.
        let path = Bundle.main.bundleURL
            .appendingPathComponent("Contents/Resources/install-launchdaemon.sh").path
        guard FileManager.default.isExecutableFile(atPath: path) else {
            throw NSError(
                domain: "Aegis",
                code: 3,
                userInfo: [NSLocalizedDescriptionKey: "Script install-launchdaemon.sh não encontrado no app"]
            )
        }

        let escaped = path.replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        let source = """
        do shell script "\"\(escaped)\"" with administrator privileges
        """
        var error: NSDictionary?
        guard let appleScript = NSAppleScript(source: source) else {
            throw NSError(domain: "Aegis", code: 4, userInfo: [NSLocalizedDescriptionKey: "Falha ao criar AppleScript"])
        }
        let result = appleScript.executeAndReturnError(&error)
        if let error {
            let msg = error[NSAppleScript.errorMessage] as? String ?? "\(error)"
            throw NSError(domain: "Aegis", code: 5, userInfo: [NSLocalizedDescriptionKey: msg])
        }
        _ = result
    }

    private func registerDaemon() throws {
        if #available(macOS 13.0, *) {
            let service = SMAppService.daemon(plistName: "\(Self.daemonPlistName).plist")
            switch service.status {
            case .enabled:
                return
            case .requiresApproval:
                if let url = URL(string: "x-apple.systempreferences:com.apple.LoginItems-Settings.extension") {
                    NSWorkspace.shared.open(url)
                }
                throw NSError(
                    domain: "Aegis",
                    code: 2,
                    userInfo: [NSLocalizedDescriptionKey: "Aprove o item em Segundo Plano nas Ajustes do Sistema"]
                )
            default:
                try service.register()
            }
        }
    }

    private func registerFailOpenAgent() throws {
        if #available(macOS 13.0, *) {
            let agent = SMAppService.agent(plistName: "\(Self.failOpenPlistName).plist")
            switch agent.status {
            case .enabled:
                return
            case .requiresApproval:
                // Non-fatal: daemon is the privileged piece; agent is safety net.
                return
            default:
                try? agent.register()
            }
        }
    }

    func unregister() throws {
        if #available(macOS 13.0, *) {
            let service = SMAppService.daemon(plistName: "\(Self.daemonPlistName).plist")
            try service.unregister()
            let agent = SMAppService.agent(plistName: "\(Self.failOpenPlistName).plist")
            try? agent.unregister()
        }
    }
}
