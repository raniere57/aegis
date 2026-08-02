import Foundation

/// Catálogo local de provedores/listas conhecidas (URLs estáveis em formato hosts/domains).
enum BlocklistCatalog {
    struct Provider: Identifiable, Hashable {
        let id: String
        let name: String
        let tagline: String
        let about: String
        let homepage: String
        let lists: [Entry]
    }

    struct Entry: Identifiable, Hashable {
        let id: String
        let name: String
        let summary: String
        let url: String
        /// Approximate size label for humans.
        let sizeHint: String
        let level: Level
        let notes: String

        enum Level: String, Hashable {
            case leve = "Leve"
            case equilibrado = "Equilibrado"
            case forte = "Forte"
            case maximo = "Máximo"
            case seguranca = "Segurança"

            var tint: String {
                switch self {
                case .leve: return "green"
                case .equilibrado: return "blue"
                case .forte: return "orange"
                case .maximo: return "red"
                case .seguranca: return "purple"
                }
            }
        }
    }

    static let providers: [Provider] = [
        Provider(
            id: "hagezi",
            name: "HaGeZi",
            tagline: "Padrão atual da comunidade · atualização frequente",
            about: """
            Listas all-in-one otimizadas (não só “juntar tudo”). Tiers do mais leve ao mais agressivo. \
            Recomendado começar pelo Multi Normal.
            """,
            homepage: "https://github.com/hagezi/dns-blocklists",
            lists: [
                Entry(
                    id: "hagezi-light",
                    name: "Multi Light",
                    summary: "Ads/trackers básicos, pouca quebra de sites.",
                    url: "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/light-onlydomains.txt",
                    sizeHint: "~100–200k",
                    level: .leve,
                    notes: "Bom para quem quer o mínimo de atrito."
                ),
                Entry(
                    id: "hagezi-multi",
                    name: "Multi Normal",
                    summary: "Melhor equilíbrio para uso diário em casa.",
                    url: "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/multi-onlydomains.txt",
                    sizeHint: "~180k",
                    level: .equilibrado,
                    notes: "Default recomendado do Aegis."
                ),
                Entry(
                    id: "hagezi-pro",
                    name: "Multi Pro",
                    summary: "Mais bloqueio; pode exigir allowlist ocasional.",
                    url: "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/pro-onlydomains.txt",
                    sizeHint: "~450–550k",
                    level: .forte,
                    notes: "Para quem aceita ajustar allowlist."
                ),
                Entry(
                    id: "hagezi-proplus",
                    name: "Multi Pro++",
                    summary: "Ainda mais agressivo que o Pro.",
                    url: "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/pro.plus-onlydomains.txt",
                    sizeHint: "~500k+",
                    level: .forte,
                    notes: "Use só se Normal/Pro não bastarem."
                ),
                Entry(
                    id: "hagezi-ultimate",
                    name: "Multi Ultimate",
                    summary: "Máximo bloqueio — espere sites/apps quebrarem.",
                    url: "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/ultimate-onlydomains.txt",
                    sizeHint: "~550k+",
                    level: .maximo,
                    notes: "Só com disposição para manter allowlist."
                ),
                Entry(
                    id: "hagezi-tif",
                    name: "Threat Intelligence Feeds",
                    summary: "Malware, phishing, scam, C2 — complemento de segurança.",
                    url: "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/tif-onlydomains.txt",
                    sizeHint: "varia",
                    level: .seguranca,
                    notes: "Pode combinar com uma Multi (Normal/Pro)."
                ),
            ]
        ),
        Provider(
            id: "oisd",
            name: "OISD",
            tagline: "Foco em poucos falsos positivos",
            about: """
            Mantenedor remove domínios que quebram sites/apps. Boa escolha “set and forget” \
            se a prioridade for estabilidade.
            """,
            homepage: "https://oisd.nl",
            lists: [
                Entry(
                    id: "oisd-small",
                    name: "OISD Small",
                    summary: "Lista curada e conservadora.",
                    url: "https://small.oisd.nl/domainswild2",
                    sizeHint: "~50–80k",
                    level: .leve,
                    notes: "Menos cobertura, menos atrito."
                ),
                Entry(
                    id: "oisd-big",
                    name: "OISD Big",
                    summary: "Cobertura ampla com baixa taxa de quebra.",
                    url: "https://big.oisd.nl/domainswild2",
                    sizeHint: "~300k",
                    level: .equilibrado,
                    notes: "Alternativa sólida ao HaGeZi Normal."
                ),
                Entry(
                    id: "oisd-nsfw",
                    name: "OISD NSFW",
                    summary: "Conteúdo adulto / NSFW.",
                    url: "https://nsfw.oisd.nl/domainswild2",
                    sizeHint: "varia",
                    level: .forte,
                    notes: "Complemento opcional (controle parental)."
                ),
            ]
        ),
        Provider(
            id: "stevenblack",
            name: "StevenBlack",
            tagline: "Hosts unificado clássico · baseline simples",
            about: """
            Combina várias fontes em um arquivo hosts. Estável e amplamente usado; \
            menos “moderno” que HaGeZi/OISD, mas previsível.
            """,
            homepage: "https://github.com/StevenBlack/hosts",
            lists: [
                Entry(
                    id: "sb-base",
                    name: "Unified hosts (base)",
                    summary: "Ads + malware + tracking básicos.",
                    url: "https://raw.githubusercontent.com/StevenBlack/hosts/master/hosts",
                    sizeHint: "~80–120k",
                    level: .leve,
                    notes: "Bom ponto de partida minimalista."
                ),
                Entry(
                    id: "sb-fakenews-gambling",
                    name: "Base + fakenews + gambling",
                    summary: "Base com extras de fake news e apostas.",
                    url: "https://raw.githubusercontent.com/StevenBlack/hosts/master/alternates/fakenews-gambling/hosts",
                    sizeHint: "~100k+",
                    level: .equilibrado,
                    notes: "Alternates oficiais do repositório."
                ),
            ]
        ),
        Provider(
            id: "1hosts",
            name: "1Hosts",
            tagline: "Listas hosts enxutas e ativas",
            about: "Projeto badmojr/1Hosts com variantes Lite / Pro / Xtra.",
            homepage: "https://github.com/badmojr/1Hosts",
            lists: [
                Entry(
                    id: "1hosts-lite",
                    name: "1Hosts Lite",
                    summary: "Versão leve.",
                    url: "https://raw.githubusercontent.com/badmojr/1Hosts/master/Lite/hosts.txt",
                    sizeHint: "menor",
                    level: .leve,
                    notes: "Pouca cobertura relativa."
                ),
                Entry(
                    id: "1hosts-xtra",
                    name: "1Hosts Xtra",
                    summary: "Versão mais completa do projeto.",
                    url: "https://raw.githubusercontent.com/badmojr/1Hosts/master/Xtra/hosts.txt",
                    sizeHint: "maior",
                    level: .forte,
                    notes: "Substitui o antigo Pro (URL 404)."
                ),
            ]
        ),
    ]
}
