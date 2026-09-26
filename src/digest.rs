use std::fmt;
use std::io::{self, Read, Write};
use std::path::Path;
use std::str::FromStr;

use anyhow::{Result, bail};
use sha2::Digest as _;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sha256([u8; 32]);

impl Sha256 {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(sha2::Sha256::digest(bytes).into())
    }

    pub fn of_file(path: &Path) -> io::Result<Self> {
        let mut file = std::fs::File::open(path)?;
        Ok(copy_hashed(&mut file, &mut io::sink())?.1)
    }

    /// The first 12 hex digits, enough to tell pins apart in `status` output.
    pub fn short(&self) -> String {
        self.to_string()[..12].to_owned()
    }
}

/// Copies `reader` to `writer`, returning the byte count and the sha256 of what was copied.
pub fn copy_hashed(reader: &mut impl Read, writer: &mut impl Write) -> io::Result<(u64, Sha256)> {
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&buf[..n]);
        writer.write_all(&buf[..n])?;
        total += n as u64;
    }
    Ok((total, Sha256(hasher.finalize().into())))
}

impl FromStr for Sha256 {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("`{s}` is not a sha256 hash (expected 64 hex digits)");
        }
        let mut bytes = [0u8; 32];
        for (byte, pair) in bytes.iter_mut().zip(s.as_bytes().chunks_exact(2)) {
            let pair = std::str::from_utf8(pair).expect("ASCII checked above");
            *byte = u8::from_str_radix(pair, 16).expect("hex checked above");
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn round_trips_and_normalizes_case() {
        let digest: Sha256 = EMPTY.to_uppercase().parse().unwrap();
        assert_eq!(digest.to_string(), EMPTY);
        assert_eq!(digest.short(), "e3b0c44298fc");
    }

    #[test]
    fn hashes_bytes_and_streams_identically() {
        assert_eq!(Sha256::of_bytes(b"").to_string(), EMPTY);
        let data = vec![7u8; 200_000];
        let mut out = Vec::new();
        let (n, digest) = copy_hashed(&mut data.as_slice(), &mut out).unwrap();
        assert_eq!(n, 200_000);
        assert_eq!(out, data);
        assert_eq!(digest, Sha256::of_bytes(&data));
    }

    #[test]
    fn rejects_wrong_length_and_non_hex() {
        assert!("abc".parse::<Sha256>().is_err());
        assert!(EMPTY.replace('e', "g").parse::<Sha256>().is_err());
    }
}
