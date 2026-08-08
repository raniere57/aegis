import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var state: AppState
    @State private var selectedTab = 0

    var body: some View {
        TabView(selection: $selectedTab) {
            GeneralSettingsView()
                .tabItem { Label("Geral", systemImage: "gear") }
                .tag(0)
            ListsSettingsView()
                .tabItem { Label("Listas", systemImage: "list.bullet") }
                .tag(1)
            RecentBlocksView()
                .tabItem { Label("Bloqueios", systemImage: "shield.lefthalf.filled") }
                .tag(2)
            AllowlistSettingsView()
                .tabItem { Label("Allowlist", systemImage: "checkmark.seal") }
                .tag(3)
            AdvancedSettingsView()
                .tabItem { Label("Avançado", systemImage: "wrench") }
                .tag(4)
        }
        .frame(minWidth: 520, minHeight: 420)
        .environmentObject(state)
    }
}

struct GeneralSettingsView: View {
    @EnvironmentObject private var state: AppState

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if let reason = state.dnsBypassReason {
                    SettingsChrome.callout(
                        title: "Por que ainda vejo ads?",
                        systemImage: "exclamationmark.triangle.fill",
                        tint: .orange,
                        body: reason
                            + "\n\nNextDNS filtra na nuvem; o Aegis só vê o que chega em 127.0.0.1. "
                            + "Desligue DNS da VPN, use split tunnel, ou desative DoH no browser."
                    )
                } else if state.connected && !state.systemDNSActive {
                    SettingsChrome.callout(
                        title: "Filtro ainda não está no DNS do Mac",
                        systemImage: "network.slash",
                        tint: .orange,
                        body: "O daemon está no ar, mas o Wi‑Fi não aponta para 127.0.0.1 — por isso 0 consultas. Use “Ativar filtro” no menu da barra."
                    )
                }

                if !state.connected {
                    SettingsChrome.callout(
                        title: "Daemon offline",
                        systemImage: "bolt.slash.fill",
                        tint: .orange,
                        body: "Clique em Atualizar status. Se continuar offline, use Ajustes → Avançado → Reparar serviço."
                    )
                } else if state.dnsEffective {
                    SettingsChrome.callout(
                        title: "DNS pelo Aegis",
                        systemImage: "checkmark.shield.fill",
                        tint: .green,
                        body: "Tráfego DNS passando pelo Aegis. Use o toggle do menu para ligar/desligar."
                    )
                }

                if let err = state.lastError {
                    SettingsChrome.callout(
                        title: "Erro",
                        systemImage: "xmark.octagon.fill",
                        tint: .red,
                        body: err
                    )
                }

                SettingsChrome.sectionTitle("Status")
                SettingsChrome.card {
                    VStack(spacing: 10) {
                        SettingsChrome.statusRow(
                            label: "Daemon",
                            value: state.connected ? "Conectado" : "Offline",
                            valueColor: state.connected ? .green : .orange
                        )
                        Divider()
                        SettingsChrome.statusRow(
                            label: "Filtro",
                            value: state.filterActive ? "Ligado" : "Desligado",
                            valueColor: state.filterActive ? .green : .secondary
                        )
                        Divider()
                        SettingsChrome.statusRow(
                            label: "DNS no Wi‑Fi/Ethernet",
                            value: state.systemDNSActive ? "Aegis (127.0.0.1)" : "Não"
                        )
                        Divider()
                        SettingsChrome.statusRow(
                            label: "DNS efetivo do Mac",
                            value: state.dnsEffective ? "Aegis" : "Outro (bypass)",
                            valueColor: state.dnsEffective ? .green : .orange
                        )
                        Divider()
                        SettingsChrome.statusRow(label: "Versão", value: state.version)
                        Divider()
                        SettingsChrome.statusRow(label: "Consultas", value: "\(state.queries)")
                        Divider()
                        SettingsChrome.statusRow(label: "Bloqueados", value: "\(state.blocked)")
                        Divider()
                        SettingsChrome.statusRow(label: "Domínios na lista", value: "\(state.domainCount)")
                        Divider()
                        SettingsChrome.statusRow(label: "Lista atualizada", value: state.listUpdatedLabel)
                    }
                }

                Button {
                    Task { await state.refresh() }
                } label: {
                    Text("Atualizar status")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.visible)
    }
}

struct AllowlistSettingsView: View {
    @EnvironmentObject private var state: AppState
    @State private var domains: [String] = []
    @State private var newDomain = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                SettingsChrome.callout(
                    title: "Allowlist",
                    systemImage: "checkmark.seal.fill",
                    tint: .accentColor,
                    body: "Domínios que nunca serão bloqueados (ex.: banco, trabalho)."
                )

                SettingsChrome.sectionTitle("Adicionar")
                SettingsChrome.card {
                    HStack(spacing: 8) {
                        TextField("dominio.com", text: $newDomain)
                            .textFieldStyle(.roundedBorder)
                            .onSubmit { Task { await addDomain() } }
                        Button("Adicionar") {
                            Task { await addDomain() }
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(newDomain.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }

                SettingsChrome.sectionTitle("Domínios (\(domains.count))")
                if domains.isEmpty {
                    Text("Nenhum domínio na allowlist.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(domains, id: \.self) { d in
                        HStack {
                            Text(d)
                                .font(.body.monospaced())
                            Spacer()
                            Button(role: .destructive) {
                                Task {
                                    domains = (try? await state.client.allowlistRemove(d)) ?? domains
                                }
                            } label: {
                                Image(systemName: "trash")
                            }
                            .buttonStyle(.borderless)
                        }
                        .padding(10)
                        .background(
                            RoundedRectangle(cornerRadius: 8)
                                .fill(Color(nsColor: .controlBackgroundColor))
                        )
                    }
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.visible)
        .task {
            domains = (try? await state.client.allowlistList()) ?? []
        }
    }

    private func addDomain() async {
        let d = newDomain.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !d.isEmpty else { return }
        domains = (try? await state.client.allowlistAdd(d)) ?? domains
        newDomain = ""
    }
}

struct AdvancedSettingsView: View {
    @EnvironmentObject private var state: AppState
    @State private var upstreamText = "1.1.1.1:53\n1.0.0.1:53"
    @State private var saveMessage: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                SettingsChrome.callout(
                    title: "Uso diário",
                    systemImage: "hand.raised.fill",
                    tint: .secondary,
                    body: "Não mexa aqui no dia a dia. Use o toggle “Ativar filtro” no menu da barra — ativar usa o Aegis como DNS; desativar ou sair restaura o DNS anterior."
                )

                SettingsChrome.sectionTitle("Upstream")
                SettingsChrome.card {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("Para onde o Aegis pergunta o que não bloqueou. Cloudflare por padrão — um endereço por linha.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                        SettingsChrome.insetEditor(text: $upstreamText, minHeight: 80)
                        Button {
                            saveUpstreams()
                        } label: {
                            Text("Salvar upstreams")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(!state.connected)
                        if let saveMessage {
                            Text(saveMessage)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }

                SettingsChrome.sectionTitle("Emergência")
                SettingsChrome.card {
                    VStack(alignment: .leading, spacing: 8) {
                        Button {
                            state.dnsManager.restoreDNS()
                            Task { await state.refresh() }
                        } label: {
                            Label("Restaurar DNS do sistema agora", systemImage: "arrow.uturn.backward")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                        Text("Só se a internet ficou estranha e o toggle não reverteu.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                SettingsChrome.sectionTitle("Técnico")
                SettingsChrome.card {
                    VStack(alignment: .leading, spacing: 12) {
                        SettingsChrome.statusRow(
                            label: "Socket",
                            value: state.client.activeSocketPath
                                ?? FileManager.default.homeDirectoryForCurrentUser
                                .appendingPathComponent(".aegis/aegis.sock").path
                        )
                        Divider()
                        HStack {
                            Text("Serviço launchd")
                                .foregroundStyle(.secondary)
                            Spacer()
                            Label(state.daemonServiceStatus, systemImage: launchdIcon)
                                .foregroundStyle(launchdColor)
                        }
                        .font(.body)

                        HStack(spacing: 8) {
                            Button {
                                Task {
                                    do {
                                        try state.serviceManager.registerIfNeeded()
                                        await state.refresh()
                                    } catch {
                                        state.lastError = error.localizedDescription
                                    }
                                }
                            } label: {
                                Text("Registrar serviço")
                                    .frame(maxWidth: .infinity)
                            }
                            .buttonStyle(.bordered)

                            Button(role: .destructive) {
                                try? state.serviceManager.unregister()
                                Task { await state.refresh() }
                            } label: {
                                Text("Remover")
                                    .frame(maxWidth: .infinity)
                            }
                            .buttonStyle(.bordered)
                        }

                        Button {
                            Task {
                                do {
                                    try state.serviceManager.repairRegistration()
                                    state.lastError = "Serviço re-registrado. Se pedir, aprove em Segundo Plano."
                                    await state.refresh()
                                } catch {
                                    state.lastError = error.localizedDescription
                                }
                            }
                        } label: {
                            Text("Reparar serviço (após update / reboot)")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        Text("Use se o daemon ficar offline depois de atualizar o app ou reiniciar o Mac.")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.visible)
        .task {
            await state.refresh()
            if let cfg = try? await state.client.getConfig(),
               let upstream = cfg["upstream"] as? [String: Any],
               let servers = upstream["servers"] as? [String] {
                upstreamText = servers.joined(separator: "\n")
            }
        }
    }

    private var launchdColor: Color {
        let s = state.daemonServiceStatus.lowercased()
        if s.contains("ativo") || s.contains("enabled") || s.contains("running") {
            return .green
        }
        if s.contains("não") || s.contains("not") || s.contains("disabled") || s.contains("offline") {
            return .orange
        }
        return .secondary
    }

    private var launchdIcon: String {
        let s = state.daemonServiceStatus.lowercased()
        if s.contains("ativo") || s.contains("enabled") || s.contains("running") {
            return "checkmark.circle.fill"
        }
        return "circle.dashed"
    }

    private func saveUpstreams() {
        let servers = upstreamText
            .split(separator: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        Task {
            do {
                try await state.client.patchConfig(["upstream": ["servers": servers]])
                saveMessage = "Upstreams salvos."
            } catch {
                saveMessage = error.localizedDescription
            }
        }
    }
}
