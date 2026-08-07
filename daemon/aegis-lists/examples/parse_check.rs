//! Ad-hoc parser check against a real list file: cargo run -p aegis-lists --example parse_check -- <file>
fn main() {
    let path = std::env::args().nth(1).expect("usage: parse_check <file>");
    let text = std::fs::read_to_string(&path).expect("read list");
    let domains = aegis_lists::normalize_list_text(&text);
    println!("entradas: {}", domains.len());
    for probe in ["google.com", "nytimes.com", "amazon.com", "reddit.com", "plus", "facebook.com"] {
        if domains.iter().any(|d| d == probe) {
            println!("  !! BLOQUEARIA {probe}");
        }
    }
}
