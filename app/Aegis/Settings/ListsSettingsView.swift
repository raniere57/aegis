import SwiftUI

struct ListsSettingsView: View {
    @EnvironmentObject private var state: AppState
    @State private var urls: [String] = []
    @State private var sources: [ListSourceStat] = []
    @State private var listCount = 0
    @State private var uniqueDomains = 0
    @State private var sumDomainCounts = 0
    @State private var autoUpdate = true
    @State private var intervalHours = 24
    @State private var newURL = ""
    @State private var busyURL: String?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let reason = state.dnsBypassReason {
                        SettingsChrome.callout(
                            title: "Ads ainda aparecem?",
                            systemImage: "exclamationmark.triangle.fill",
                            tint: .orange,
                            body: reason
                                + "\n\nNextDNS funciona com VPN porque filtra na nuvem: as queries vão até os servidores deles. "
                                + "O Aegis filtra só no Mac — se a VPN mandar o DNS para outro lugar, o Aegis nem vê a query. "
                                + "Também desative DNS-over-HTTPS no browser."
                        )
                    }

                    SettingsChrome.sectionTitle("Resumo")
                    SettingsChrome.card {
                        VStack(spacing: 8) {
                            HStack {
                                Text("Listas adicionadas")
                                Spacer()
                                Text("\(listCount)")
                                    .font(.title2.weight(.semibold).monospacedDigit())
                            }
                            HStack {
                                Text("Itens (soma das listas)")
                                Spacer()
                                Text(AppState.compact(UInt64(sumDomainCounts)))
                                    .font(.title2.weight(.semibold).monospacedDigit())
                            }
                            HStack {
                                Text("Únicos após merge")
                                Spacer()
                                Text(AppState.compact(UInt64(uniqueDomains > 0 ? uniqueDomains : state.domainCount)))
                                    .font(.title3.weight(.medium).monospacedDigit())
                                    .foregroundStyle(.secondary)
                            }
                            Text("A soma conta cada lista; “únicos” é o que o Aegis usa de fato (sem duplicatas).")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        }
                    }

                    SettingsChrome.sectionTitle("Atualização")
                    SettingsChrome.card {
                        VStack(alignment: .leading, spacing: 10) {
                            LabeledContent("Última atualização") {
                                Text(state.listUpdatedLabel)
                                    .multilineTextAlignment(.trailing)
                            }
                            Toggle("Atualização automática", isOn: $autoUpdate)
                                .onChange(of: autoUpdate) { _, v in
                                    Task {
                                        try? await state.client.patchConfig(["lists": ["auto_update": v]])
                                    }
                                }
                            Picker("Intervalo", selection: $intervalHours) {
                                Text("6 h").tag(6)
                                Text("12 h").tag(12)
                                Text("24 h").tag(24)
                            }
                            .pickerStyle(.segmented)
                            .onChange(of: intervalHours) { _, v in
                                Task {
                                    try? await state.client.patchConfig(["lists": ["interval_hours": v]])
                                }
                            }

                            Button {
                                Task {
                                    await state.updateLists()
                                    await reload()
                                }
                            } label: {
                                HStack {
                                    if state.listUpdating {
                                        ProgressView().controlSize(.small)
                                    }
                                    Text(state.listUpdating ? "Atualizando…" : "Atualizar listas agora")
                                }
                                .frame(maxWidth: .infinity)
                            }
                            .buttonStyle(.borderedProminent)
                            .disabled(!state.connected || state.listUpdating)

                            if let msg = state.listUpdateMessage {
                                Text(msg)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }

                    SettingsChrome.sectionTitle("Provedores")
                    Text("Escolha um para ver as listas e adicionar.")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    ForEach(BlocklistCatalog.providers) { provider in
                        NavigationLink(value: provider) {
                            HStack {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(provider.name).font(.headline)
                                    Text(provider.tagline)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(2)
                                }
                                Spacer()
                                let n = activeCount(in: provider)
                                if n > 0 {
                                    Text("\(n)")
                                        .font(.caption.weight(.bold))
                                        .padding(6)
                                        .background(Circle().fill(Color.accentColor.opacity(0.2)))
                                }
                                Image(systemName: "chevron.right")
                                    .font(.caption)
                                    .foregroundStyle(.tertiary)
                            }
                            .padding(12)
                            .background(RoundedRectangle(cornerRadius: 10).fill(Color(nsColor: .controlBackgroundColor)))
                        }
                        .buttonStyle(.plain)
                    }

                    SettingsChrome.sectionTitle("Ativas (\(listCount))")
                    if urls.isEmpty {
                        Text("Nenhuma lista.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    ForEach(urls, id: \.self) { url in
                        let stat = sources.first(where: { $0.url == url })
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(displayName(for: url)).font(.subheadline.weight(.medium))
                                Text("\(AppState.compact(UInt64(stat?.domainCount ?? 0))) itens · \(url)")
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(2)
                                if let err = stat?.lastError, !err.isEmpty {
                                    Label(err, systemImage: "exclamationmark.triangle.fill")
                                        .font(.caption2)
                                        .foregroundStyle(.red)
                                        .lineLimit(2)
                                }
                            }
                            Spacer()
                            Button(role: .destructive) {
                                Task { await remove(url) }
                            } label: {
                                Image(systemName: "trash")
                            }
                            .buttonStyle(.borderless)
                            .disabled(busyURL == url || !state.connected)
                        }
                        .padding(10)
                        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
                    }

                    SettingsChrome.sectionTitle("URL personalizada")
                    SettingsChrome.card {
                        HStack {
                            TextField("https://…", text: $newURL)
                                .textFieldStyle(.roundedBorder)
                            Button("Add") {
                                Task {
                                    let u = newURL.trimmingCharacters(in: .whitespacesAndNewlines)
                                    guard !u.isEmpty else { return }
                                    await add(u)
                                    newURL = ""
                                }
                            }
                            .disabled(!state.connected || newURL.isEmpty)
                        }
                    }
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .scrollIndicators(.visible)
            .navigationTitle("Listas")
            .navigationDestination(for: BlocklistCatalog.Provider.self) { provider in
                ProviderDetailView(
                    provider: provider,
                    activeURLs: urls,
                    connected: state.connected,
                    busyURL: busyURL,
                    onAdd: { await add($0) },
                    onRemove: { await remove($0) }
                )
            }
        }
        .task {
            await reload()
            await state.refresh()
        }
    }

    private func activeCount(in provider: BlocklistCatalog.Provider) -> Int {
        provider.lists.filter { urls.contains($0.url) }.count
    }

    private func displayName(for url: String) -> String {
        for p in BlocklistCatalog.providers {
            if let e = p.lists.first(where: { $0.url == url }) {
                return "\(p.name) · \(e.name)"
            }
        }
        return "Personalizada"
    }

    private func reload() async {
        guard let info = try? await state.client.listsList() else { return }
        urls = info.urls
        sources = info.sources
        listCount = info.listCount
        uniqueDomains = info.uniqueDomains
        sumDomainCounts = info.sumDomainCounts
        autoUpdate = info.autoUpdate
        intervalHours = info.intervalHours
    }

    private func add(_ url: String) async {
        busyURL = url
        defer { busyURL = nil }
        do {
            try await state.client.listsAdd(url)
            await reload()
            await state.updateLists()
            await reload()
        } catch {
            state.listUpdateMessage = error.localizedDescription
        }
    }

    private func remove(_ url: String) async {
        busyURL = url
        defer { busyURL = nil }
        do {
            try await state.client.listsRemove(url)
            await reload()
            await state.updateLists()
            await reload()
        } catch {
            state.listUpdateMessage = error.localizedDescription
        }
    }
}

struct ProviderDetailView: View {
    let provider: BlocklistCatalog.Provider
    let activeURLs: [String]
    let connected: Bool
    let busyURL: String?
    let onAdd: (String) async -> Void
    let onRemove: (String) async -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text(provider.about)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Link("Abrir site do provedor", destination: URL(string: provider.homepage)!)
                    .font(.caption)

                ForEach(provider.lists) { entry in
                    CatalogEntryRow(
                        entry: entry,
                        isActive: activeURLs.contains(entry.url),
                        connected: connected,
                        busy: busyURL == entry.url,
                        onAdd: { Task { await onAdd(entry.url) } },
                        onRemove: { Task { await onRemove(entry.url) } }
                    )
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.visible)
        .navigationTitle(provider.name)
    }
}

private struct CatalogEntryRow: View {
    let entry: BlocklistCatalog.Entry
    let isActive: Bool
    let connected: Bool
    let busy: Bool
    let onAdd: () -> Void
    let onRemove: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(entry.name).font(.subheadline.weight(.semibold))
                Spacer()
                LevelBadge(level: entry.level)
            }
            Text(entry.summary).font(.caption)
            Text("\(entry.sizeHint) · \(entry.notes)")
                .font(.caption2)
                .foregroundStyle(.secondary)
            HStack {
                if isActive {
                    Label("Ativa", systemImage: "checkmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.green)
                    Spacer()
                    Button("Remover", role: .destructive, action: onRemove)
                        .disabled(!connected || busy)
                } else {
                    Spacer()
                    Button("Adicionar", action: onAdd)
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                        .disabled(!connected || busy)
                }
                if busy { ProgressView().controlSize(.small) }
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
        .opacity(busy ? 0.7 : 1)
    }
}

private struct LevelBadge: View {
    let level: BlocklistCatalog.Entry.Level

    var body: some View {
        Text(level.rawValue)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(color.opacity(0.18), in: Capsule())
            .foregroundStyle(color)
    }

    private var color: Color {
        switch level {
        case .leve: return .green
        case .equilibrado: return .blue
        case .forte: return .orange
        case .maximo: return .red
        case .seguranca: return .purple
        }
    }
}
