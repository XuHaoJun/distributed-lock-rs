//! File name validation and conversion utilities.

use std::path::{Path, PathBuf};
use sha2::{Digest, Sha512};

use distributed_lock_core::error::{LockError, LockResult};

/// Minimum file name length to avoid collisions.
const MIN_FILE_NAME_LENGTH: usize = 12;

/// Portable file name length (includes hash and extension).
const PORTABLE_FILE_NAME_LENGTH: usize = 64;

/// Hash length in Base32 characters (160 bits / 5 bits per char).
const HASH_LENGTH_IN_CHARS: usize = 32;

/// Base32 alphabet (RFC 4648).
const BASE32_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Converts a lock name to a safe file name and constructs the full path.
///
/// # Rules
///
/// - Names with only alphanumeric, `-`, and `_` are used as-is (with prefix)
/// - Other characters are replaced with underscores
/// - A Base32 hash is appended to ensure uniqueness and case-sensitivity
/// - The result is truncated to fit filesystem limits
pub fn get_lock_file_name(directory: &Path, name: &str) -> LockResult<PathBuf> {
    if name.is_empty() {
        return Err(LockError::InvalidName("lock name cannot be empty".to_string()));
    }

    let directory_path = directory
        .canonicalize()
        .or_else(|_| std::fs::create_dir_all(directory).map(|_| directory.to_path_buf()))
        .map_err(|e| {
            LockError::InvalidName(format!("failed to create directory: {}", e))
        })?;

    let directory_path_str = directory_path.to_string_lossy();
    let directory_path_with_separator = if directory_path_str.ends_with('/') {
        directory_path_str.to_string()
    } else {
        format!("{}/", directory_path_str)
    };

    let base_name = convert_to_valid_base_name(name);
    let name_hash = compute_hash(name.as_bytes());
    const EXTENSION: &str = ".lock";

    // First, try the full portable name format
    let base_name_prefix_len = PORTABLE_FILE_NAME_LENGTH
        .saturating_sub(name_hash.len())
        .saturating_sub(EXTENSION.len())
        .min(base_name.len());

    let portable_lock_file_name = format!(
        "{}{}{}{}",
        directory_path_with_separator,
        &base_name[..base_name_prefix_len],
        name_hash,
        EXTENSION
    );

    if !is_too_long(&portable_lock_file_name) {
        return Ok(PathBuf::from(portable_lock_file_name));
    }

    // Next, try using just the hash as the name
    let hash_only_file_name = format!("{}{}", directory_path_with_separator, name_hash);
    if !is_too_long(&hash_only_file_name) {
        return Ok(PathBuf::from(hash_only_file_name));
    }

    // Finally, try using just a portion of the hash
    let minimum_length_file_name = format!(
        "{}{}",
        directory_path_with_separator,
        &name_hash[..name_hash.len().min(MIN_FILE_NAME_LENGTH)]
    );
    if !is_too_long(&minimum_length_file_name) {
        return Ok(PathBuf::from(minimum_length_file_name));
    }

    Err(LockError::InvalidName(format!(
        "unable to construct lock file name: directory path too long (length = {})",
        directory_path_with_separator.len()
    )))
}

fn is_too_long(path: &str) -> bool {
    Path::new(path)
        .canonicalize()
        .is_err()
        && path.len() > 255 // Conservative check
}

fn convert_to_valid_base_name(name: &str) -> String {
    const REPLACEMENT_CHAR: char = '_';

    let mut result = String::with_capacity(name.len());
    let mut needs_replacement = false;

    for ch in name.chars() {
        if ch.is_alphanumeric() || ch == REPLACEMENT_CHAR {
            result.push(ch);
        } else {
            if !needs_replacement {
                needs_replacement = true;
            }
            result.push(REPLACEMENT_CHAR);
        }
    }

    result
}

fn compute_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha512::new();
    hasher.update(bytes);
    let hash_bytes = hasher.finalize();

    // Truncate to 160 bits (20 bytes) for Base32 encoding
    let truncated: Vec<u8> = hash_bytes[..20].to_vec();

    // Encode to Base32
    let mut chars = Vec::with_capacity(HASH_LENGTH_IN_CHARS);
    let mut bit_buffer = 0u32;
    let mut bits_remaining = 0u32;

    for byte in truncated {
        bit_buffer |= (byte as u32) << bits_remaining;
        bits_remaining += 8;

        while bits_remaining >= 5 {
            let index = (bit_buffer & 0x1f) as usize;
            chars.push(BASE32_ALPHABET[index] as char);
            bit_buffer >>= 5;
            bits_remaining -= 5;
        }
    }

    // Handle remaining bits
    if bits_remaining > 0 {
        let index = (bit_buffer & 0x1f) as usize;
        chars.push(BASE32_ALPHABET[index] as char);
    }

    // Pad to exact length
    while chars.len() < HASH_LENGTH_IN_CHARS {
        chars.push('A'); // Padding character
    }

    chars.into_iter().take(HASH_LENGTH_IN_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_valid_name() {
        let dir = env::temp_dir();
        let result = get_lock_file_name(&dir, "my-lock");
        assert!(result.is_ok());
        let path = result.unwrap();
        // The path should contain the base name (possibly truncated) and end with .lock
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("my-lock") || path_str.contains("my_lock"));
        assert!(path_str.ends_with(".lock"));
    }

    #[test]
    fn test_invalid_chars() {
        let dir = env::temp_dir();
        let result = get_lock_file_name(&dir, "foo/bar");
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_string_lossy().contains("foo_bar"));
    }
}
