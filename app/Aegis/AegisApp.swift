import SwiftUI
import AppKit

@main
struct AegisApp: App {
    @StateObject private var appState = AppState()
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        MenuBarExtra {
            MenuBarView()
                .environmentObject(appState)
                .onAppear { AppDelegate.appState = appState }
        } label: {
            Image(systemName: appState.menuIconName)
                .symbolRenderingMode(.hierarchical)
        }
        .menuBarExtraStyle(.menu)

        // Dedicated window (not Settings scene) — reliable z-order for menu-bar apps.
        Window("Ajustes", id: "settings") {
            SettingsView()
                .environmentObject(appState)
                .frame(minWidth: 560, minHeight: 480)
                .onAppear { AppDelegate.appState = appState }
        }
        .windowResizability(.contentSize)
        .defaultSize(width: 620, height: 520)
        .commandsRemoved()
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    /// Set from AegisApp so terminate can fail-open DNS.
    static weak var appState: AppState?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(windowWillClose(_:)),
            name: NSWindow.willCloseNotification,
            object: nil
        )
    }

    func applicationWillTerminate(_ notification: Notification) {
        // Reboot / Quit: release DNS so next boot is never stuck on 127.0.0.1 without aegisd.
        if Thread.isMainThread {
            AppDelegate.appState?.prepareForTermination()
        } else {
            DispatchQueue.main.sync {
                AppDelegate.appState?.prepareForTermination()
            }
        }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        SettingsWindowBridge.shared.revealExisting()
        return true
    }

    @objc private func windowWillClose(_ note: Notification) {
        DispatchQueue.main.async {
            SettingsWindowBridge.shared.restoreAccessoryIfIdle()
        }
    }
}

/// Brings the Ajustes window to the active Space and front of other apps.
@MainActor
final class SettingsWindowBridge {
    static let shared = SettingsWindowBridge()

    func open(_ openWindow: OpenWindowAction) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        openWindow(id: "settings")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.08) {
            self.revealExisting()
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
            self.revealExisting()
        }
    }

    func revealExisting() {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)

        let candidates = NSApp.windows.filter { window in
            let typeName = String(describing: type(of: window))
            if typeName.contains("StatusBar") || typeName.contains("MenuBar") { return false }
            guard window.styleMask.contains(.titled) else { return false }
            return window.title == "Ajustes"
                || (window.identifier?.rawValue.contains("settings") ?? false)
                || window.isKeyWindow
                || (window.isVisible && window.canBecomeKey)
        }

        let target = candidates.first(where: { $0.title == "Ajustes" })
            ?? candidates.first

        if let window = target {
            window.collectionBehavior.insert(.moveToActiveSpace)
            window.level = .floating
            window.makeKeyAndOrderFront(nil)
            window.orderFrontRegardless()
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
                window.level = .normal
            }
        }
        NSApp.activate(ignoringOtherApps: true)
    }

    func restoreAccessoryIfIdle() {
        let open = NSApp.windows.contains { window in
            let typeName = String(describing: type(of: window))
            return window.isVisible
                && window.styleMask.contains(.titled)
                && !typeName.contains("StatusBar")
        }
        if !open {
            NSApp.setActivationPolicy(.accessory)
        }
    }
}
