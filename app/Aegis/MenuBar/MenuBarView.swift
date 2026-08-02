import SwiftUI
import AppKit

struct MenuBarView: View {
    @EnvironmentObject private var state: AppState
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Toggle(isOn: Binding(
                get: { state.filterActive },
                set: { newValue in
                    Task { await state.toggleEnabled(newValue) }
                }
            )) {
                Text("Ativar filtro")
            }
            .disabled(!state.connected && !state.filterActive)

            Text(state.statusLine)
                .font(.caption)
                .foregroundStyle(state.dnsBypassReason != nil ? .orange : .secondary)

            if state.dnsBypassReason != nil {
                Text("VPN/DNS externo contorna o filtro")
                    .font(.caption2)
                    .foregroundStyle(.orange)
            }

            Divider()

            Button("Atualizar listas") {
                Task { await state.updateLists() }
            }
            .disabled(!state.connected)

            Button("Atualizar status") {
                Task { await state.refresh() }
            }

            Divider()

            Button("Ajustes…") {
                SettingsWindowBridge.shared.open(openWindow)
            }

            Button("Sair") {
                Task { await state.quitApp() }
            }
        }
        .onAppear {
            Task { await state.refresh() }
        }
    }
}
