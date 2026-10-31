use port_rs::{index::Index, query::Matcher};
use std::{hint::black_box, time::Instant};
fn main() {
    let mut data = String::new();
    for i in 0..50_000 {
        let name = format!("package{i:05}");
        let body = format!(
            "name {name} version 1.2.3 revision 1 categories {{devel net}} portdir devel/{name} description {{A representative package description}} depends_lib {{port:zlib port:openssl}}"
        );
        data.push_str(&format!("{name} {}\n{body}\n", body.len() + 1));
    }
    let start = Instant::now();
    for _ in 0..5 {
        black_box(Index::parse(black_box(&data)).unwrap());
    }
    println!(
        "Parse 50,000 synthetic ports ({:.1} MiB): {:.2?}/iteration",
        data.len() as f64 / 1_048_576.0,
        start.elapsed() / 5
    );
    let index = Index::parse(&data).unwrap();
    let matcher = Matcher::new("*package123*", false, false, false).unwrap();
    let start = Instant::now();
    for _ in 0..100 {
        black_box(
            index
                .ports()
                .values()
                .filter(|p| matcher.matches(&p.name))
                .count(),
        );
    }
    println!(
        "Search 50,000 loaded ports: {:.2?}/iteration",
        start.elapsed() / 100
    );
    let start = Instant::now();
    black_box(port_rs::query::lookup(&index, "PACKAGE49999").unwrap());
    println!(
        "First fallback lookup (including lazy cache): {:.2?}",
        start.elapsed()
    );
    for label in ["PACKAGE49999", "not-a-port"] {
        let start = Instant::now();
        for _ in 0..1000 {
            black_box(port_rs::query::lookup(black_box(&index), black_box(label)).ok());
        }
        println!(
            "Warm lookup {label} in 50,000 ports: {:.2?}/lookup",
            start.elapsed() / 1000
        );
    }
}
