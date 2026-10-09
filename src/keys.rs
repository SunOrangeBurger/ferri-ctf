use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::fmt;
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

type HmacSha256 = Hmac<Sha256>;

/// Purpose-separated keys derived from SERVER_SECRET. A MAC made under one key
/// is never valid under another, so flags, sessions and CSRF cannot be confused.
pub struct Keys {
    pub flag: [u8; 32],
    pub session: [u8; 32],
    pub csrf: [u8; 32],
}

impl Keys {
    pub fn derive(secret: &[u8]) -> Self {
        Self {
            flag: subkey(secret, b"ferrisctf/v1/flag"),
            session: subkey(secret, b"ferrisctf/v1/session"),
            csrf: subkey(secret, b"ferrisctf/v1/csrf"),
        }
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        self.flag.zeroize();
        self.session.zeroize();
        self.csrf.zeroize();
    }
}

impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Keys(<redacted>)")
    }
}

fn subkey(secret: &[u8], label: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(secret).expect("any key length");
    mac.update(label);
    let mut out = [0u8; 32];
    out.copy_from_slice(&mac.finalize().into_bytes());
    out
}

/// hex(HMAC-SHA256(key, parts concatenated))
pub fn mac_hex(key: &[u8], parts: &[&[u8]]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("any key length");
    for p in parts {
        mac.update(p);
    }
    hex::encode(mac.finalize().into_bytes())
}

/// Constant-time string equality (different lengths compare unequal).
pub fn ct_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subkeys_are_distinct_and_deterministic() {
        let a = Keys::derive(&[7u8; 32]);
        let b = Keys::derive(&[7u8; 32]);
        let c = Keys::derive(&[8u8; 32]);
        assert_eq!(a.session, b.session);
        assert_ne!(a.session, a.csrf);
        assert_ne!(a.session, a.flag);
        assert_ne!(a.csrf, a.flag);
        assert_ne!(a.session, c.session);
    }

    #[test]
    fn ct_eq_basics() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
    }
}
