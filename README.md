# Aegis

Filtro DNS nativo para macOS. Bloqueia anúncios e trackers no seu Mac, sem nuvem e sem limite de queries — uma alternativa local ao NextDNS free.

**SwiftUI MenuBarExtra** para a interface + **daemon Rust** para o trabalho pesado.

> ⚠️ **v0.1 — projeto pessoal, sem assinatura da Apple.** Mexe no DNS do sistema e roda um LaunchDaemon como root. Leia [Segurança](#segurança) e [Se algo der errado](#se-algo-der-errado) antes de instalar.

---

## Como funciona

Seu Mac normalmente pergunta o IP de cada domínio direto para o DNS do provedor. O Aegis se coloca no meio:

```
apps do Mac
    │ "qual o IP de ads.exemplo.com?"
    ▼
127.0.0.1:53 ──► aegisd
                   │
                   ├─ é localhost / .local?  ──► deixa passar
                   ├─ está na allowlist?     ──► deixa passar
                   ├─ está na blocklist?     ──► NXDOMAIN (morreu aqui)
                   ├─ está no cache?         ──► responde na hora
                   └─ senão                  ──► pergunta ao upstream (1.1.1.1)
```

Domínio bloqueado recebe `NXDOMAIN` — o app que pediu simplesmente não encontra o servidor de anúncios. Bloquear um domínio bloqueia todos os subdomínios dele.

Bloquear pelo nome perguntado não basta: trackers se escondem atrás de um CNAME de primeira parte (`metrics.site.com` → `tracker.exemplo.net`). O Aegis também checa a cadeia de CNAME da resposta antes de entregá-la.

A interface é só um cliente: fala com o daemon por um Unix socket, um JSON por linha.

```
MenuBarExtra  ──socket──►  aegisd (LaunchDaemon)
                             ├─ allowlist (HashSet)
                             ├─ blocklist (FST + mmap)
                             ├─ cache (LRU)
                             ├─ upstream DNS53
                             └─ auto-update das listas
```

### Por que FST + mmap

Uma blocklist séria tem milhões de domínios. Guardar isso como `Vec<String>` custaria centenas de MB de RAM em um processo que fica ligado o dia inteiro.

O Aegis compila a lista em um [FST](https://docs.rs/fst) (autômato finito determinístico) e faz `mmap` do arquivo. As páginas ficam no cache de arquivos do sistema, não no heap do processo — o RSS fica perto do working set, não do tamanho total.

Medido de verdade, não estimado:

| Cenário | RSS do `aegisd` |
|---|---|
| Lista default (~180k domínios), em repouso | **~2 MB** |
| HaGeZi Ultimate (3,4M domínios), após 5.000 queries | **~23 MB** |

Os 23 MB são quase todos páginas do FST tocadas pelas consultas — memória limpa e respaldada por arquivo, que o sistema recupera sob pressão sem swap. O [docs/BUDGET.md](docs/BUDGET.md) ainda diz "≤ 15 MB com 3,4M"; isso é uma meta, não uma medição, e a meta não está sendo cumprida.

Trocar de lista usa `ArcSwap`: a lista nova é montada em paralelo e entra com um swap de ponteiro atômico. Nenhuma query trava durante a atualização.

### O hot path é sagrado

Toda query DNS passa pelo mesmo caminho, e ele é proibido de fazer: SQLite, HTTP, parse de TOML, ou log por query. Metas em [docs/BUDGET.md](docs/BUDGET.md) — p50 de bloqueio abaixo de 0,5 ms.

---

## Fail-open: a decisão de design mais importante

Um filtro de DNS quebrado não deixa você com anúncios. Deixa você **sem internet**. Todo o projeto é construído em volta disso:

| Camada | O que faz |
|---|---|
| Daemon | Blocklist ausente, corrompida ou vazia → encaminha tudo em vez de bloquear |
| App (liveness) | Detecta daemon morto no boot, ao sair e a cada 5 s → restaura o DNS |
| App (resolução) | 25 falhas seguidas de upstream com o DNS em loopback → restaura. Estar vivo não é estar saudável: o daemon pode responder `status` perfeitamente com a internet morta |
| LaunchAgent | Watchdog a cada 30 s: se o DNS aponta para o Aegis e o daemon não responde → restaura |
| Script | [`restore-dns.sh`](packaging/scripts/restore-dns.sh) — botão de pânico manual |
| Desinstalação | [`uninstall.sh`](packaging/scripts/uninstall.sh) restaura o DNS **antes** de remover qualquer coisa |

O DNS anterior é salvo em `dns-backup.json` antes de qualquer alteração. Perder o bloqueio por um tempo é sempre melhor que perder a internet.

Também existe um guard contra listas que encolhem: se o rebuild vier com menos de 50% dos domínios anteriores **e algum download tiver falhado**, a lista antiga é mantida. Uma URL morta não derruba seu bloqueio. E se todas as listas baixarem com 200 OK mas nenhuma linha for reconhecida — o caso do portal cativo devolvendo HTML — a lista anterior também é mantida, em vez de gravar um filtro vazio e reportar sucesso.

---

## Instalação

Baixe o `.dmg` mais recente em [Releases](https://github.com/raniere57/aegis/releases), arraste o Aegis para `/Applications` e então:

```bash
sudo bash /Applications/Aegis.app/Contents/Resources/install-launchdaemon.sh
```

Esse passo é obrigatório e não é opcional-por-conveniência: ele copia o `aegisd` para `/usr/local/libexec/aegis` (`root:wheel`, modo 755) e aponta o LaunchDaemon para lá. `/Applications` é modo 775 do grupo `admin`, então um daemon executado de dentro do bundle poderia ser substituído por qualquer processo seu e reexecutado como uid 0 no próximo boot.

Como o build é ad-hoc, o Gatekeeper reclama na primeira abertura — clique direito → **Abrir**.

Para desinstalar:

```bash
sudo bash /Applications/Aegis.app/Contents/Resources/uninstall.sh
```

---

## Requisitos

- macOS 14+
- Rust 1.75+
- Xcode / Swift 5.9+

## Rodando em modo dev (sem root)

Modo dev usa `~/.aegis/` e a porta 53553. Não toca no DNS do sistema, não precisa de senha.

```bash
cd daemon
cargo run -p aegisd -- --dev
```

Em outro terminal:

```bash
cargo run -p aegis-ctl -- --dev status
```

Testando uma query de verdade:

```bash
dig @127.0.0.1 -p 53553 example.com
```

> Porta 53553, não 5353 — o macOS reserva a 5353 para mDNS/Bonjour.

O app Swift:

```bash
cd app && swift run
```

Ele encontra o socket sozinho: tenta `/var/run/aegis.sock` (privilegiado) e cai para `~/.aegis/aegis.sock` (dev).

## Build do .app e do DMG

```bash
./packaging/scripts/build-dmg.sh
```

Sai em `dist/Aegis.app` e `dist/Aegis-0.1.0.dmg`.

Para usar a porta 53 de verdade, o daemon precisa rodar privilegiado. O app instala um LaunchDaemon clássico em `/Library/LaunchDaemons/` via prompt de administrador — mais confiável entre reboots do que o `SMAppService`, que costuma sair com `EX_CONFIG` depois de uma re-assinatura ad-hoc.

---

## Listas de bloqueio

O default é o **HaGeZi Multi Normal** (~180k domínios), um bom equilíbrio para uso diário.

| Lista | Quando usar |
|---|---|
| HaGeZi Multi Light | Mínimo de atrito |
| **HaGeZi Multi Normal** (default) | Melhor equilíbrio |
| HaGeZi Multi Pro / Pro++ | Mais agressivo, exige allowlist ocasional |
| HaGeZi Ultimate | Máximo — espere quebrar coisa |
| HaGeZi TIF | Malware, phishing, C2 — complemento de segurança |
| OISD Big | Poucos falsos positivos, "set and forget" |
| StevenBlack | Baseline clássico e previsível |

**Prefira uma lista boa a uma dúzia.** Elas se sobrepõem muito: `Light ⊂ Normal ⊂ Pro ⊂ Pro++ ⊂ Ultimate`. Adicionar as cinco baixa o mesmo conteúdo cinco vezes sem bloquear quase nada a mais.

O Aegis usa as variantes `wildcard/*-onlydomains.txt` do HaGeZi (domínios puros) servidas pelo `raw.githubusercontent.com`. Evite `cdn.jsdelivr.net` com `@latest` — responde 403 e não segue mudanças de estrutura do repositório.

O parser aceita hosts (`0.0.0.0 dominio`), listas de domínios puros e regras Adblock simples (`||dominio^`). Atualização automática diária com ETag e `If-Modified-Since` — um 304 não gasta banda nem CPU.

Diretórios úteis: [hagezi/dns-blocklists](https://github.com/hagezi/dns-blocklists) · [OISD](https://oisd.nl) · [Firebog](https://firebog.net)

---

## "Por que isso parou de funcionar?"

A aba **Bloqueios** mostra os últimos domínios bloqueados, mais recentes primeiro, com contagem de repetições — e um botão **Permitir** que joga o domínio na allowlist na hora.

É um ring buffer de 256 slots no daemon, 35 KB fixos, sem disco. Nada de histórico de navegação persistido: o que passou dos 256 últimos bloqueios desaparece, e reiniciar o daemon zera. Um filtro de DNS que grava tudo que você acessa é um problema de privacidade, não um recurso.

---

## Uso pelo terminal

O `aegis-ctl` fala o mesmo protocolo que a interface:

```bash
aegis-ctl status
aegis-ctl metrics
aegis-ctl lists list
aegis-ctl allowlist add exemplo.com
aegis-ctl update-lists
```

Qualquer método do protocolo, incluindo os que não têm subcomando:

```bash
aegis-ctl raw patch_config '{"lists":{"interval_hours":12}}'
```

Protocolo completo em [docs/PROTOCOL.md](docs/PROTOCOL.md).

---

## Se algo der errado

DNS quebrado e a interface não abre:

```bash
./packaging/scripts/restore-dns.sh
```

Restaura o DNS de todas as interfaces a partir do backup e limpa o cache. Funciona com o daemon morto.

Desinstalar de vez (restaura o DNS antes de remover qualquer coisa):

```bash
sudo bash /Applications/Aegis.app/Contents/Resources/uninstall.sh
```

Adicione `--purge` para apagar também config, blocklist e logs.

Ver o que o daemon está fazendo:

```bash
tail -f /var/log/aegisd.log
```

---

## Limitações conhecidas

- **VPNs vencem.** Se a VPN impõe o DNS dela, a query nunca chega ao Aegis. A interface detecta e avisa, mas não tem como contornar — filtro local só vê o que passa pela máquina. É por isso que o NextDNS funciona com VPN e o Aegis não: eles filtram na nuvem.
- **iCloud Private Relay passa por cima de tudo.** Se você assina o iCloud+, ele vem ligado e faz o Safari resolver nomes por Oblivious DoH direto para a Apple — a query nunca chega em `127.0.0.1` e o Aegis não vê nada. Sintoma clássico: o `dig` bloqueia normalmente, mas um teste de adblock no navegador dá quase 0%. Desligue em Ajustes do Sistema → Apple Account → iCloud → Retransmissão Privada.
- **DNS-over-HTTPS no navegador também passa por cima.** Chrome e Firefox fazem DoH por conta própria. Desative nas configurações do navegador.
- **Sem DoH de saída.** O upstream é DNS53 puro, então seu provedor vê quais domínios você resolve.
- **Sem Network Extension.** Um `NEDNSProxyProvider` seria mais robusto que mexer no `networksetup`, mas exige conta paga de desenvolvedor Apple.
- **Sem assinatura/notarização.** Build ad-hoc: o Gatekeeper reclama na primeira abertura (clique direito → Abrir).
- **Arrastar o Aegis.app para o lixo não desinstala.** O daemon vive em `/usr/local/libexec/aegis` e continua rodando, mas o watchdog de fail-open some junto com o bundle. Use [`uninstall.sh`](packaging/scripts/uninstall.sh), que restaura o DNS antes de remover qualquer coisa.

## Segurança

O daemon roda como root para escutar na porta 53. Isso significa que o socket de controle é uma superfície séria: quem fala com ele pode trocar os servidores upstream, ou seja, sequestrar todo o DNS da máquina.

- O socket é `0660`, dono `root`, grupo `admin`. Contas standard, apps sandboxed e daemons de terceiros não alcançam.
- **Ainda não há verificação de credencial do processo que conecta.** Qualquer processo rodando como um usuário administrador pode controlar o daemon. Um `LOCAL_PEERCRED` por uid seria o próximo passo.
- O daemon recusa upstreams que apontem para os próprios listeners dele (laço infinito) e URLs de lista que não sejam `https://` — root busca essas URLs.
- Respostas do upstream só entram no cache com o transaction ID batendo com o da pergunta e o bit QR ligado. O socket também é `connect()`ado, então o kernel já descarta datagramas de outra origem.
- O binário privilegiado fica em `/usr/local/libexec/aegis` (`root:wheel`), fora do bundle gravável pelo grupo `admin`. O instalador **não** re-assina nada como root: `codesign --force --sign -` com privilégio transformaria um binário trocado em um binário carregável, que é o passo da escalada, não a mitigação.

Achou algo? Abra uma issue.

## Estrutura

```
daemon/
  aegis-core/    config, métricas, cache, FST, proxy DNS, tipos do IPC
  aegis-lists/   download, normalização, compilação, metadados SQLite
  aegisd/        binário do daemon
  aegis-ctl/     CLI
app/Aegis/       SwiftUI MenuBarExtra
packaging/       LaunchDaemon/Agent, scripts de build e recuperação
docs/            arquitetura, protocolo, budget de performance
```

Mais detalhes em [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Licença

MIT — ver [LICENSE](LICENSE).
