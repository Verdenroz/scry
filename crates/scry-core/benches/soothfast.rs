//! Hot paths measured by soothfast: content hashing runs on every file of
//! every sync, and remote-URL normalization on every repo detection.

use soothfast::{bench, fixture, keep};

soothfast::bench_main!();

/// Deterministic pseudo-random bytes (seeded LCG, no rand dep).
#[fixture]
fn bytes_n(n: usize) -> Vec<u8> {
    let mut x: u64 = 0x5DEE_CE66_D111_1111;
    (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 33) as u8
        })
        .collect()
}

#[fixture]
fn remote_url() -> String {
    "ssh://git@github.com:22/Verdenroz/scry.git".to_string()
}

#[bench(
    group = "sync",
    setup_sized = bytes_n,
    sizes(4096, 65536, 1048576),
    complexity = "n",
    alloc = 0,
    covers = "scry_core::hashing::hash_bytes"
)]
fn bench_hash_bytes(input: &[u8]) {
    keep(scry_core::hashing::hash_bytes(keep(input)));
}

#[bench(
    group = "repo",
    setup = remote_url,
    alloc = 5,
    covers = "scry_core::repo::normalize_remote_url"
)]
fn bench_normalize_remote_url(url: &str) {
    keep(scry_core::repo::normalize_remote_url(keep(url)));
}
