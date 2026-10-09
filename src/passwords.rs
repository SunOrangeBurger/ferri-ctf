use std::{sync::Arc, time::Duration};

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

use crate::errors::{AppError, AppResult};

// OWASP minimum for Argon2id: 19 MiB, 2 iterations, 1 lane. Explicit, so a crate
// default change can never silently alter the cost of new hashes.
const MEM_KIB: u32 = 19_456;
const ITERATIONS: u32 = 2;
const LANES: u32 = 1;

fn hasher() -> Argon2<'static> {
    let params = Params::new(MEM_KIB, ITERATIONS, LANES, None).expect("valid argon2 params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn hash_blocking(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(hasher().hash_password(password.as_bytes(), &salt)?.to_string())
}

fn verify_blocking(password: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => hasher().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// Argon2 behind a concurrency limit (spec 9.5): work runs on the blocking pool,
/// at most `permits` at once; extra callers queue briefly, then get a 503.
pub struct Passwords {
    permits: Arc<Semaphore>,
    queue_timeout: Duration,
    /// Valid hash of a random password, verified when the username is unknown so
    /// response time does not reveal whether an account exists (spec 9.3).
    dummy: String,
}

impl Passwords {
    pub fn new() -> Self {
        Self::with_limits(num_cpus::get().max(1) * 2, Duration::from_secs(5))
    }

    pub fn with_limits(permits: usize, queue_timeout: Duration) -> Self {
        let random = SaltString::generate(&mut OsRng);
        let dummy = hash_blocking(random.as_str()).expect("argon2 self-test hash");
        Self { permits: Arc::new(Semaphore::new(permits)), queue_timeout, dummy }
    }

    async fn run<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let permit = tokio::time::timeout(self.queue_timeout, self.permits.clone().acquire_owned())
            .await
            .map_err(|_| AppError::ServiceUnavailable)?
            .map_err(|_| AppError::internal("password semaphore closed"))?;
        let out = tokio::task::spawn_blocking(move || {
            let _permit = permit; // released when the hash finishes, not when the caller does
            f()
        })
        .await?;
        Ok(out)
    }

    pub async fn hash(&self, password: String) -> AppResult<String> {
        let pw = Zeroizing::new(password);
        self.run(move || hash_blocking(&pw))
            .await?
            .map_err(|e| AppError::internal(format!("argon2 hash failed: {e}")))
    }

    /// `stored = None` means "no such user": verifies against the dummy hash, then
    /// returns false, so unknown and known usernames cost the same.
    pub async fn verify(&self, password: String, stored: Option<String>) -> AppResult<bool> {
        let pw = Zeroizing::new(password);
        let (phc, real) = match stored {
            Some(h) => (h, true),
            None => (self.dummy.clone(), false),
        };
        let ok = self.run(move || verify_blocking(&pw, &phc)).await?;
        Ok(ok && real)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hash_and_verify() {
        let p = Passwords::new();
        let h1 = p.hash("hunter2hunter2".into()).await.unwrap();
        let h2 = p.hash("hunter2hunter2".into()).await.unwrap();
        assert!(h1.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{h1}");
        assert_ne!(h1, h2, "salts must differ");
        assert!(p.verify("hunter2hunter2".into(), Some(h1.clone())).await.unwrap());
        assert!(!p.verify("hunter2hunter3".into(), Some(h1)).await.unwrap());
    }

    #[tokio::test]
    async fn unknown_user_and_garbage_hashes_never_verify() {
        let p = Passwords::new();
        assert!(!p.verify("anything".into(), None).await.unwrap());
        assert!(!p.verify("anything".into(), Some("not-a-phc-string".into())).await.unwrap());
        assert!(!p.verify("anything".into(), Some(String::new())).await.unwrap());
    }

    #[tokio::test]
    async fn saturated_pool_fails_with_503() {
        let p = Passwords::with_limits(1, Duration::from_millis(50));
        let _held = p.permits.clone().acquire_owned().await.unwrap();
        assert!(matches!(p.hash("whatever1".into()).await, Err(AppError::ServiceUnavailable)));
    }
}
