//! PostgreSQL advisory lock key encoding.

use distributed_lock_core::error::{LockError, LockResult};
use sha2::{Digest, Sha256};

/// Key for PostgreSQL advisory locks.
///
/// Advisory locks use either a single 64-bit key or a pair of 32-bit keys.
/// These represent different key spaces and do not overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostgresAdvisoryLockKey {
    /// Single 64-bit key.
    Single(i64),
    /// Pair of 32-bit keys.
    Pair(i32, i32),
}

impl PostgresAdvisoryLockKey {
    /// Maximum length for ASCII encoding (9 characters).
    const MAX_ASCII_LENGTH: usize = 9;
    /// Bits per ASCII character (7 bits).
    const ASCII_CHAR_BITS: u32 = 7;
    /// Maximum ASCII value (127).
    const MAX_ASCII_VALUE: u32 = (1 << Self::ASCII_CHAR_BITS) - 1;
    /// Hash string length (16 hex chars for i64).
    const HASH_STRING_LENGTH: usize = 16;
    /// Hash part length (8 hex chars for i32).
    const HASH_PART_LENGTH: usize = 8;
    /// Hash string separator.
    const HASH_STRING_SEPARATOR: char = ',';

    /// Create a key from a string name.
    ///
    /// - ASCII strings up to 9 chars are encoded directly (collision-free)
    /// - 16-char hex strings are parsed as i64
    /// - "XXXXXXXX,XXXXXXXX" format parsed as (i32, i32)
    /// - Other strings are hashed to i64 (if `allow_hashing` is true)
    pub fn from_name(name: &str, allow_hashing: bool) -> LockResult<Self> {
        if name.is_empty() {
            return Err(LockError::InvalidName("lock name cannot be empty".to_string()));
        }

        // Try ASCII encoding first
        if let Some(key) = Self::try_encode_ascii(name) {
            return Ok(Self::Single(key));
        }

        // Try parsing as hex string
        if let Some(key) = Self::try_parse_hex_string(name) {
            return Ok(key);
        }

        // Try parsing as pair format
        if let Some(key) = Self::try_parse_pair_string(name) {
            return Ok(key);
        }

        // Hash if allowed
        if allow_hashing {
            let hash = Self::hash_string(name);
            return Ok(Self::Single(hash));
        }

        Err(LockError::InvalidName(format!(
            "Name '{}' could not be encoded as a PostgresAdvisoryLockKey. Please specify allow_hashing or use one of the following formats: (1) a 0-{} character string using only ASCII characters, (2) a {} character hex string, or (3) a 2-part, {} character string of the form XXXXXXXX{}XXXXXXXX",
            name,
            Self::MAX_ASCII_LENGTH,
            Self::HASH_STRING_LENGTH,
            Self::HASH_PART_LENGTH * 2 + 1,
            Self::HASH_STRING_SEPARATOR
        )))
    }

    /// Try to encode as ASCII string (up to 9 chars).
    fn try_encode_ascii(name: &str) -> Option<i64> {
        if name.len() > Self::MAX_ASCII_LENGTH {
            return None;
        }

        let mut result = 0i64;
        for ch in name.chars() {
            let ch_val = ch as u32;
            if ch_val > Self::MAX_ASCII_VALUE {
                return None;
            }
            result = (result << Self::ASCII_CHAR_BITS) | (ch_val as i64);
        }

        // Add padding: shift by 1 (zero bit), then fill remaining with 1s
        result <<= 1;
        for _ in name.len()..Self::MAX_ASCII_LENGTH {
            result = (result << Self::ASCII_CHAR_BITS) | (Self::MAX_ASCII_VALUE as i64);
        }

        Some(result)
    }

    /// Try to parse as hex string (16 chars for i64).
    fn try_parse_hex_string(name: &str) -> Option<Self> {
        if name.len() != Self::HASH_STRING_LENGTH {
            return None;
        }

        i64::from_str_radix(name, 16)
            .ok()
            .map(Self::Single)
    }

    /// Try to parse as pair format "XXXXXXXX,XXXXXXXX".
    fn try_parse_pair_string(name: &str) -> Option<Self> {
        let parts: Vec<&str> = name.split(Self::HASH_STRING_SEPARATOR).collect();
        if parts.len() != 2 {
            return None;
        }

        let key1 = i32::from_str_radix(parts[0], 16).ok()?;
        let key2 = i32::from_str_radix(parts[1], 16).ok()?;

        Some(Self::Pair(key1, key2))
    }

    /// Hash a string to i64 using SHA-256 (taking first 8 bytes).
    fn hash_string(name: &str) -> i64 {
        let mut hasher = Sha256::new();
        hasher.update(name.as_bytes());
        let hash_bytes = hasher.finalize();

        // Take first 8 bytes and convert to i64 (little-endian)
        let mut result = 0i64;
        for i in (0..8).rev() {
            result = (result << 8) | (hash_bytes[i] as i64);
        }
        result
    }

    /// Returns true if this is a single key.
    pub fn has_single_key(&self) -> bool {
        matches!(self, Self::Single(_))
    }

    /// Gets the single key value (panics if Pair).
    pub fn key(&self) -> i64 {
        match self {
            Self::Single(k) => *k,
            Self::Pair(_, _) => panic!("key() called on Pair variant"),
        }
    }

    /// Gets the key pair (splits Single into two i32s).
    pub fn keys(&self) -> (i32, i32) {
        match self {
            Self::Single(k) => {
                // Split i64 into two i32s
                let upper = (*k >> 32) as i32;
                let lower = (*k & 0xFFFFFFFF) as i32;
                (upper, lower)
            }
            Self::Pair(k1, k2) => (*k1, *k2),
        }
    }

    /// Convert to SQL function arguments.
    pub fn to_sql_args(&self) -> String {
        match self {
            Self::Single(k) => format!("{}", k),
            Self::Pair(k1, k2) => format!("{}, {}", k1, k2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_encoding() {
        let key = PostgresAdvisoryLockKey::from_name("test", false).unwrap();
        assert!(key.has_single_key());
    }

    #[test]
    fn test_hex_encoding() {
        let hex_str = "0000000000000001";
        let key = PostgresAdvisoryLockKey::from_name(hex_str, false).unwrap();
        assert!(key.has_single_key());
        assert_eq!(key.key(), 1);
    }

    #[test]
    fn test_pair_encoding() {
        let pair_str = "00000001,00000002";
        let key = PostgresAdvisoryLockKey::from_name(pair_str, false).unwrap();
        match key {
            PostgresAdvisoryLockKey::Pair(k1, k2) => {
                assert_eq!(k1, 1);
                assert_eq!(k2, 2);
            }
            _ => panic!("Expected Pair variant"),
        }
    }

    #[test]
    fn test_hash_encoding() {
        let long_name = "this is a very long lock name that needs hashing";
        let key = PostgresAdvisoryLockKey::from_name(long_name, true).unwrap();
        assert!(key.has_single_key());
    }
}
