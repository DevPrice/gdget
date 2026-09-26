use std::fmt;
use std::str::FromStr;

use anyhow::{Result, bail};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sha256([u8; 32]);

impl Sha256 {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The first 12 hex digits, enough to tell pins apart in `status` output.
    pub fn short(&self) -> String {
        self.to_string()[..12].to_owned()
    }
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
    fn rejects_wrong_length_and_non_hex() {
        assert!("abc".parse::<Sha256>().is_err());
        assert!(EMPTY.replace('e', "g").parse::<Sha256>().is_err());
    }
}
