use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use xxhash_rust::xxh64::{Xxh64, xxh64};

pub fn hash_bytes(bytes: &[u8]) -> u64 {
    xxh64(bytes, 0)
}

pub fn hash_file(path: &Path) -> io::Result<u64> {
    let mut file = File::open(path)?;
    let mut hasher = Xxh64::new(0);
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hasher.digest());
        }
        hasher.update(&buf[..n]);
    }
}

pub fn hex(hash: u64) -> String {
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_hash_matches_bytes_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, b"scry").unwrap();
        assert_eq!(hash_file(&path).unwrap(), hash_bytes(b"scry"));
    }

    #[test]
    fn hex_is_16_lowercase_digits() {
        assert_eq!(hex(0xABCD), "000000000000abcd");
    }
}
