//! SHA-256 digests over canonical NUL-joined byte strings.
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::autonomous::waterfall::sha256_hex;

use super::error::EvaluationError;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Digest(String);

impl Digest {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(sha256_hex(bytes))
    }

    /// NUL-separated canonical form, matching `review_evidence.rs` and the
    /// managed-project journal.
    pub fn of_parts(parts: &[&[u8]]) -> Self {
        let mut buf = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                buf.push(0);
            }
            buf.extend_from_slice(part);
        }
        Self::of_bytes(&buf)
    }

    pub fn parse(value: &str) -> Result<Self, EvaluationError> {
        if value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(value.to_string()))
        } else {
            Err(EvaluationError::parse(format!(
                "digest must be 64 lowercase hex chars, got {value:?}"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn short(&self) -> &str {
        &self.0[..16]
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Digest {
    type Error = EvaluationError;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::parse(&v)
    }
}

impl From<Digest> for String {
    fn from(v: Digest) -> String {
        v.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn of_parts_is_nul_separated_sha256() {
        let a = Digest::of_parts(&[b"x", b"y"]);
        let b = Digest::of_bytes(b"x\0y");
        assert_eq!(a, b);
        assert_ne!(a, Digest::of_parts(&[b"xy"]));
        assert_eq!(a.as_str().len(), 64);
        assert_eq!(a.short().len(), 16);
    }

    #[test]
    fn parse_requires_64_lowercase_hex() {
        assert!(Digest::parse(&"a".repeat(64)).is_ok());
        assert!(Digest::parse(&"A".repeat(64)).is_err());
        assert!(Digest::parse("abc").is_err());
    }
}
