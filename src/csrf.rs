use crate::keys::{self, Keys};
use rand::{rngs::OsRng, RngCore};

pub const FORM_FIELD: &str = "csrf_token";
pub const HEADER_NAME: &str = "x-csrf-token";

/// Token = hex(nonce) "." hex(HMAC(csrf_key, binding ":" nonce)).
/// `binding` is the session id for logged-in users. A fresh nonce per issue
/// means the token bytes differ on every page render.
pub fn issue(keys: &Keys, binding: &str) -> String {
    let mut n = [0u8; 16];
    OsRng.fill_bytes(&mut n);
    let nonce = hex::encode(n);
    let mac = keys::mac_hex(&keys.csrf, &[binding.as_bytes(), b":", nonce.as_bytes()]);
    format!("{nonce}.{mac}")
}

pub fn verify(keys: &Keys, binding: &str, token: &str) -> bool {
    let Some((nonce, mac)) = token.split_once('.') else {
        return false;
    };
    if nonce.len() != 32 || mac.len() != 64 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let expected = keys::mac_hex(&keys.csrf, &[binding.as_bytes(), b":", nonce.as_bytes()]);
    keys::ct_eq(&expected, mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_binding() {
        let k = Keys::derive(&[1u8; 32]);
        let t = issue(&k, "session-a");
        assert!(verify(&k, "session-a", &t));
        assert!(!verify(&k, "session-b", &t), "token must be bound to its session");
        assert_ne!(issue(&k, "session-a"), issue(&k, "session-a"), "nonce must vary");
    }

    #[test]
    fn rejects_tampering_and_garbage() {
        let k = Keys::derive(&[1u8; 32]);
        let other = Keys::derive(&[2u8; 32]);
        let t = issue(&k, "s");
        assert!(!verify(&other, "s", &t));
        let mut bad = t.clone();
        bad.pop();
        bad.push(if t.ends_with('0') { '1' } else { '0' });
        assert!(!verify(&k, "s", &bad));
        for junk in ["", ".", "abc", "a.b", &"0".repeat(200)] {
            assert!(!verify(&k, "s", junk));
        }
    }
}
