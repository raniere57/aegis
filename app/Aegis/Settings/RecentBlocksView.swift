import SwiftUI

/// "Why was this blocked?" — the daemon keeps the last 256 blocked names in a fixed 35 KB ring,
/// so this tab costs nothing when nobody is looking at it and never touches disk.
struct RecentBlocksView: View {
    @EnvironmentObject private var state: AppState
    @State private var entries: [RecentBlock] = []
    @State private var loadError: String?
    @State private var busy = false

    private let client = DaemonClient()

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                SettingsChrome.callout(
                    title: "Bloqueios recentes",
                    systemImage: "shield.lefthalf.filled",
                    tint: .accentColor,
                    body: "Os últimos domínios bloqueados, mais recentes primeiro. "
                        + "Se algo parou de funcionar, provavelmente está aqui — "
                        + "libere com “Permitir”. Nada é gravado em disco."
                )

                if let loadError {
                    SettingsChrome.callout(
                        title: "Não foi possível ler",
                        systemImage: "exclamationmark.triangle.fill",
                        tint: .orange,
                        body: loadError
                    )
                }

                HStack {
                    SettingsChrome.sectionTitle("Domínios (\(entries.count))")
                    Spacer()
                    Button("Atualizar") { Task { await load() } }
                        .disabled(busy)
                    Button("Limpar") { Task { await clear() } }
                        .disabled(busy || entries.isEmpty)
                }

                if entries.isEmpty {
                    Text("Nenhum bloqueio registrado desde que o daemon subiu.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(entries) { entry in
                        SettingsChrome.card {
                            HStack(spacing: 10) {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(entry.domain)
                                        .font(.body.monospaced())
                                        .textSelection(.enabled)
                                    Text(subtitle(for: entry))
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                Spacer()
                                Button("Permitir") {
                                    Task { await allow(entry.domain) }
                                }
                                .disabled(busy)
                            }
                        }
                    }
                }
            }
            .padding(16)
        }
        .task { await load() }
    }

    private func subtitle(for entry: RecentBlock) -> String {
        let when = RelativeDateTimeFormatter()
        when.unitsStyle = .short
        let date = Date(timeIntervalSince1970: TimeInterval(entry.atUnix))
        let ago = when.localizedString(for: date, relativeTo: Date())
        return entry.hits > 1 ? "\(entry.hits)× · \(ago)" : ago
    }

    private func load() async {
        busy = true
        defer { busy = false }
        do {
            entries = try await client.recentBlocked()
            loadError = nil
        } catch {
            loadError = error.localizedDescription
        }
    }

    private func clear() async {
        busy = true
        defer { busy = false }
        _ = try? await client.recentClear()
        await load()
    }

    private func allow(_ domain: String) async {
        busy = true
        defer { busy = false }
        do {
            _ = try await client.allowlistAdd(domain)
            await state.refresh()
            await load()
        } catch {
            loadError = error.localizedDescription
        }
    }
}
