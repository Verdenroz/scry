//! Recall of the binary coarse pass against exact cosine KNN on a real
//! index, swept over coarse_k. Usage:
//!   cargo run --release -p scry-core --example refiner_curve -- <index.db> [repo_key]
//! Copy the live DB first (`sqlite3 index.db "VACUUM INTO 'copy.db'"`) so
//! WAL pages are included and the server is never touched.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use scry_core::config::Config;
use scry_core::store::Store;

const K: usize = 50;
const SAMPLES: usize = 200;
const COARSE_KS: &[usize] = &[100, 200, 400, 800, 1600];

fn main() {
    let mut args = std::env::args().skip(1);
    let db = args.next().expect("index.db path");
    let repo_key = args.next();
    let config = Config::load(None).unwrap();
    let store = Store::open(
        std::path::Path::new(&db),
        &config.embedding.model,
        config.embedding.dim,
    )
    .unwrap();
    let repo_id = repo_key.and_then(|key| store.repo_id(&key).unwrap());
    let samples = store.sample_chunk_vectors(repo_id, SAMPLES).unwrap();
    println!("repo {:?}  samples {}  k {}", repo_id, samples.len(), K);

    let mut exact_time = Duration::ZERO;
    let exact: Vec<HashSet<i64>> = samples
        .iter()
        .map(|(id, query)| {
            let started = Instant::now();
            let hits = store.dense_search_exact(repo_id, query, K + 1).unwrap();
            exact_time += started.elapsed();
            hits.iter()
                .map(|h| h.chunk_id)
                .filter(|c| c != id)
                .take(K)
                .collect()
        })
        .collect();
    println!(
        "exact      recall 1.000  {:>6.1}ms/query",
        exact_time.as_secs_f64() * 1000.0 / samples.len() as f64
    );

    for &coarse_k in COARSE_KS {
        let mut time = Duration::ZERO;
        let mut overlap = 0usize;
        for ((id, query), truth) in samples.iter().zip(&exact) {
            let started = Instant::now();
            let hits = store
                .dense_search_coarse(repo_id, query, K + 1, coarse_k + 1)
                .unwrap();
            time += started.elapsed();
            overlap += hits
                .iter()
                .map(|h| h.chunk_id)
                .filter(|c| c != id)
                .take(K)
                .filter(|c| truth.contains(c))
                .count();
        }
        println!(
            "coarse {:>5} recall {:.3}  {:>6.1}ms/query",
            coarse_k,
            overlap as f64 / (samples.len() * K) as f64,
            time.as_secs_f64() * 1000.0 / samples.len() as f64
        );
    }
}
