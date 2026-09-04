//! Recall of the binary coarse pass against exact cosine KNN on a real
//! index, swept over coarse_k. Usage:
//!   cargo run --release -p scry-core --example refiner_curve -- <index.db> [repo_key]
//! Copy the live DB first (`sqlite3 index.db "VACUUM INTO 'copy.db'"`) so
//! WAL pages are included and the server is never touched.

use std::time::{Duration, Instant};

use scry_core::config::Config;
use scry_core::store::Store;

const K: usize = 50;
const K_SHOWN: usize = 10;
const SAMPLES: usize = 200;
const COARSE_KS: &[usize] = &[100, 200, 400, 800, 1600];

fn main() {
    let mut args = std::env::args().skip(1);
    let db = args.next().expect("index.db path");
    let repo_key = args.next();
    let config = Config::load(None).unwrap();
    let opened = Instant::now();
    let store = Store::open(
        std::path::Path::new(&db),
        &config.embedding.model,
        config.embedding.dim,
    )
    .unwrap();
    println!(
        "open (with any migration) {:.1}ms",
        opened.elapsed().as_secs_f64() * 1000.0
    );
    let repo_id = repo_key.and_then(|key| store.repo_id(&key).unwrap());
    let samples = store.sample_chunk_vectors(repo_id, SAMPLES).unwrap();
    println!("repo {:?}  samples {}  k {}", repo_id, samples.len(), K);

    let mut exact_time = Duration::ZERO;
    let exact: Vec<Truth> = samples
        .iter()
        .map(|(id, query)| {
            let started = Instant::now();
            let hits = store.dense_search_exact(repo_id, query, K + 1).unwrap();
            exact_time += started.elapsed();
            let others: Vec<f64> = hits
                .iter()
                .filter(|h| h.chunk_id != *id)
                .map(|h| h.distance)
                .take(K)
                .collect();
            Truth {
                at_shown: others[K_SHOWN - 1],
                at_k: others[K - 1],
            }
        })
        .collect();
    println!(
        "exact       recall@50 1.000  recall@10 1.000  {:>6.1}ms/query",
        exact_time.as_secs_f64() * 1000.0 / samples.len() as f64
    );
    for &coarse_k in COARSE_KS {
        report(
            &format!("coarse {coarse_k:>5}"),
            &samples,
            &exact,
            |query| {
                store
                    .dense_search_coarse(repo_id, query, K + 1, coarse_k + 1)
                    .unwrap()
            },
        );
    }
}

/// Recall counts a hit when its distance is within the exact k-th distance,
/// so ties between duplicate vectors do not read as misses.
struct Truth {
    at_shown: f64,
    at_k: f64,
}

const TIE: f64 = 1e-6;

fn report(
    label: &str,
    samples: &[(i64, Vec<f32>)],
    exact: &[Truth],
    mut search: impl FnMut(&[f32]) -> Vec<scry_core::store::DenseHit>,
) {
    let mut time = Duration::ZERO;
    let (mut overlap, mut overlap_shown) = (0usize, 0usize);
    for ((id, query), truth) in samples.iter().zip(exact) {
        let started = Instant::now();
        let hits = search(query);
        time += started.elapsed();
        let got: Vec<f64> = hits
            .iter()
            .filter(|h| h.chunk_id != *id)
            .map(|h| h.distance)
            .take(K)
            .collect();
        overlap += got.iter().filter(|d| **d <= truth.at_k + TIE).count();
        overlap_shown += got
            .iter()
            .take(K_SHOWN)
            .filter(|d| **d <= truth.at_shown + TIE)
            .count();
    }
    println!(
        "{label} recall@50 {:.3}  recall@10 {:.3}  {:>6.1}ms/query",
        overlap as f64 / (samples.len() * K) as f64,
        overlap_shown as f64 / (samples.len() * K_SHOWN) as f64,
        time.as_secs_f64() * 1000.0 / samples.len() as f64
    );
}
