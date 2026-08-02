//! Hot-path microbench for blocklist lookup.

use aegis_core::trie::Blocklist;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_lookup(c: &mut Criterion) {
    let mut bl = Blocklist::new();
    for i in 0..50_000 {
        bl.insert(&format!("ads{i}.tracker.example.com"));
    }
    bl.insert("blocked.example.org");

    c.bench_function("blocklist_hit", |b| {
        b.iter(|| {
            black_box(bl.contains("blocked.example.org"));
        })
    });
    c.bench_function("blocklist_subdomain_hit", |b| {
        b.iter(|| {
            black_box(bl.contains("x.y.blocked.example.org"));
        })
    });
    c.bench_function("blocklist_miss", |b| {
        b.iter(|| {
            black_box(bl.contains("www.apple.com"));
        })
    });
}

criterion_group!(benches, bench_lookup);
criterion_main!(benches);
