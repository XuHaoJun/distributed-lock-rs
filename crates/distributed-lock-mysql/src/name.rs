//! MySQL lock name encoding and validation.
//!
//! MySQL lock names have a maximum length of 64 characters and must be lowercase.
//! Invalid names are escaped/hashed to ensure validity.

use sha2::{Digest, Sha512};

/// Maximum length for MySQL lock names.
pub const MAX_NAME_LENGTH: usize = 64;

/// Encodes a lock name to be safe for MySQL GET_LOCK function.
///
/// MySQL lock names:
/// - Must be at most 64 characters
/// - Must not contain uppercase letters
/// - Should not be empty
///
/// If the name doesn't meet these requirements, it will be transformed:
/// - Empty names become "__empty__"
/// - Names are converted to lowercase
/// - If still too long, a hash is computed and truncated/prefixed
pub fn encode_lock_name(name: &str) -> String {
    to_safe_name(name, MAX_NAME_LENGTH, convert_to_valid_name, compute_hash)
}

/// Converts a name to a valid MySQL lock name base.
fn convert_to_valid_name(name: &str) -> String {
    if name.is_empty() {
        "__empty__".to_string()
    } else {
        name.to_lowercase()
    }
}

/// Computes a Base32 hash of the input bytes.
///
/// Uses SHA512 truncated to 160 bits (32 Base32 characters).
/// This provides good collision resistance while keeping names reasonably short.
#[allow(clippy::disallowed_methods)]
fn compute_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha512::new();
    hasher.update(bytes);
    let hash_bytes = hasher.finalize();

    // Truncate to 160 bits (20 bytes) for 32 Base32 characters
    const BASE32_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    const HASH_LENGTH_IN_CHARS: usize = 160 / 5; // 5 bits per Base32 char

    let mut chars = Vec::with_capacity(HASH_LENGTH_IN_CHARS);
    let mut byte_index = 0;
    let mut bit_buffer = 0u32;
    let mut bits_remaining = 0;

    for _ in 0..HASH_LENGTH_IN_CHARS {
        if bits_remaining < 5 && byte_index < 20 {
            bit_buffer |= (hash_bytes[byte_index] as u32) << bits_remaining;
            bits_remaining += 8;
            byte_index += 1;
        }

        let char_index = (bit_buffer & 31) as usize;
        chars.push(BASE32_ALPHABET[char_index] as char);
        bit_buffer >>= 5;
        bits_remaining -= 5;
    }

    chars.into_iter().collect()
}

/// Converts a name to a safe MySQL lock name with the given constraints.
fn to_safe_name(
    name: &str,
    max_name_length: usize,
    convert_to_valid_name: impl Fn(&str) -> String,
    hash: impl Fn(&[u8]) -> String,
) -> String {
    let valid_base_lock_name = convert_to_valid_name(name);

    // If the converted name is valid and fits, use it directly
    if valid_base_lock_name == name
        && valid_base_lock_name.len() <= max_name_length
        && !valid_base_lock_name.is_empty()
        && valid_base_lock_name.chars().all(|c| c.is_lowercase())
    {
        return name.to_string();
    }

    let name_hash = hash(name.as_bytes());

    if name_hash.len() >= max_name_length {
        // Hash is too long, truncate it
        name_hash[..max_name_length].to_string()
    } else {
        // Prefix the hash with as much of the original name as possible
        let prefix_len = max_name_length - name_hash.len();
        let prefix = valid_base_lock_name
            .chars()
            .take(prefix_len)
            .collect::<String>();
        format!("{}{}", prefix, name_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_names_passthrough() {
        assert_eq!(encode_lock_name("valid_lock"), "valid_lock");
        assert_eq!(encode_lock_name("another-valid"), "another-valid");
        assert_eq!(encode_lock_name("123numbers"), "123numbers");
    }

    #[test]
    fn test_uppercase_conversion() {
        assert_eq!(encode_lock_name("UPPERCASE"), "uppercase");
    }

    #[test]
    fn test_empty_name() {
        assert_eq!(encode_lock_name(""), "__empty__");
    }

    #[test]
    fn test_long_name_truncation() {
        let long_name = "a".repeat(100);
        let encoded = encode_lock_name(&long_name);
        assert!(encoded.len() <= MAX_NAME_LENGTH);
        assert!(encoded.chars().all(|c| c.is_lowercase()));
    }

    #[test]
    fn test_special_characters() {
        let name = "lock with spaces & symbols!";
        let encoded = encode_lock_name(name);
        assert!(encoded.len() <= MAX_NAME_LENGTH);
        assert!(encoded.chars().all(|c| c.is_lowercase()));
        // Should start with "lockwithspaces" and end with hash
        assert!(encoded.starts_with("lockwithspaces"));
    }
}
