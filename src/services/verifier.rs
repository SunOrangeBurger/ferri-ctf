use rand::{rngs::OsRng, RngCore};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::keys;

/// Generate 16 cryptographically random bytes, hex-encoded (32 hex characters).
pub fn generate_salt() -> String {
    let mut buf = [0u8; 16];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Compute HMAC-SHA256(secret, salt || flag)
pub fn hash_flag(flag_key: &[u8], salt: &str, flag: &str) -> String {
    let trimmed = Zeroizing::new(flag.trim().to_owned());
    keys::mac_hex(flag_key, &[salt.as_bytes(), trimmed.as_bytes()])
}

/// Verify a submitted flag against stored HMAC in constant time.
/// Returns true if correct, false otherwise.
pub fn verify_flag(flag_key: &[u8], salt: &str, submitted: &str, stored_hmac: &str) -> bool {
    let computed = hash_flag(flag_key, salt, submitted);
    computed.as_bytes().ct_eq(stored_hmac.as_bytes()).into()
}

/// Compute HMAC-SHA256(flag_key, submitted) for audit logging without storing plaintext flag.
pub fn hash_submission(flag_key: &[u8], submitted: &str) -> String {
    let trimmed = Zeroizing::new(submitted.trim().to_owned());
    keys::mac_hex(flag_key, &[trimmed.as_bytes()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flag_verification_and_trimming() {
        let key = [42u8; 32];
        let salt = generate_salt();
        assert_eq!(salt.len(), 32);

        let flag = "ferris{s3cr3t_fl4g_123}";
        let hmac = hash_flag(&key, &salt, flag);

        // Exact match
        assert!(verify_flag(&key, &salt, flag, &hmac));

        // Surrounding whitespace is trimmed
        assert!(verify_flag(&key, &salt, "  ferris{s3cr3t_fl4g_123}\n", &hmac));
        assert!(verify_flag(&key, &salt, "\tferris{s3cr3t_fl4g_123} ", &hmac));

        // Wrong flag
        assert!(!verify_flag(&key, &salt, "ferris{wrong}", &hmac));

        // Case-sensitive inside flag
        assert!(!verify_flag(&key, &salt, "FERRIS{s3cr3t_fl4g_123}", &hmac));

        // Wrong key or wrong salt
        let other_key = [99u8; 32];
        assert!(!verify_flag(&other_key, &salt, flag, &hmac));
        let other_salt = generate_salt();
        assert!(!verify_flag(&key, &other_salt, flag, &hmac));
    }

    #[test]
    fn test_hash_submission() {
        let key = [42u8; 32];
        let h1 = hash_submission(&key, "ferris{flag}");
        let h2 = hash_submission(&key, "  ferris{flag} ");
        assert_eq!(h1, h2);
        assert_ne!(h1, hash_submission(&key, "different"));
    }
}
