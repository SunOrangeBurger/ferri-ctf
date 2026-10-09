use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    async_trait,
    extract::{ConnectInfo, FromRequestParts},
    http::request::Parts,
};

#[derive(Clone, Copy, Debug)]
pub struct Limit {
    pub max: usize,
    pub window: Duration,
}

// Spec 9.4. Per-team limits (submit, download) and the join limit will reuse this limiter.
pub const LOGIN: Limit = Limit { max: 10, window: Duration::from_secs(60) };
pub const REGISTER: Limit = Limit { max: 10, window: Duration::from_secs(60) };
pub const ADMIN_LOGIN: Limit = Limit { max: 3, window: Duration::from_secs(300) };
pub const TEAM_JOIN_PREVIEW: Limit = Limit { max: 10, window: Duration::from_secs(60) };
pub const CHALLENGE_SUBMIT: Limit = Limit { max: 20, window: Duration::from_secs(60) };
pub const CHALLENGE_DOWNLOAD: Limit = Limit { max: 30, window: Duration::from_secs(60) };

/// Longest window of any limit; entries idle for longer than this can be dropped.
const MAX_WINDOW: Duration = Duration::from_secs(300);
/// Hard cap on tracked keys. When full of live entries the limiter fails closed.
const MAX_KEYS: usize = 50_000;

/// In-memory sliding-window limiter keyed by (bucket, key). Denied attempts are not
/// recorded, so a blocked client is free again once its oldest hit ages out.
#[derive(Default)]
pub struct RateLimiter {
    hits: Mutex<HashMap<(&'static str, String), VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if the attempt is allowed (and records it).
    pub fn check(&self, bucket: &'static str, key: &str, limit: Limit) -> bool {
        self.check_at(Instant::now(), bucket, key, limit)
    }

    pub fn check_at(&self, now: Instant, bucket: &'static str, key: &str, limit: Limit) -> bool {
        let mut map = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        let id = (bucket, key.to_owned());

        if map.len() >= MAX_KEYS && !map.contains_key(&id) {
            map.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < MAX_WINDOW));
            if map.len() >= MAX_KEYS {
                return false;
            }
        }

        let q = map.entry(id).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= limit.window) {
            q.pop_front();
        }
        if q.len() >= limit.max {
            return false;
        }
        q.push_back(now);
        true
    }

    /// Housekeeping; called from the periodic background task.
    pub fn purge(&self) {
        let now = Instant::now();
        let mut map = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < MAX_WINDOW));
    }
}

/// IPv4 mapped addresses collapse to IPv4, and IPv6 clients are keyed by /64, so a
/// client cannot dodge limits by rotating through addresses in its own prefix.
pub fn ip_key(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
    }
}

/// The peer address of the TCP connection. Never read from forwarded headers: there is
/// no trusted proxy in this deployment, so such headers would be attacker-controlled.
/// Without connection info (tests only) everything shares one "unknown" key, which is
/// the stricter failure mode.
pub struct ClientIp(pub String);

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Infallible> {
        let key = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| ip_key(c.0.ip()))
            .unwrap_or_else(|| "unknown".to_string());
        Ok(Self(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L3: Limit = Limit { max: 3, window: Duration::from_secs(60) };

    #[test]
    fn allows_up_to_limit_then_blocks() {
        let rl = RateLimiter::new();
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(rl.check_at(t0, "login", "a", L3));
        }
        assert!(!rl.check_at(t0, "login", "a", L3));
        assert!(!rl.check_at(t0 + Duration::from_secs(59), "login", "a", L3), "still inside the window");
    }

    #[test]
    fn window_slides_and_keys_are_isolated() {
        let rl = RateLimiter::new();
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(rl.check_at(t0, "login", "a", L3));
        }
        assert!(rl.check_at(t0, "login", "b", L3), "other key unaffected");
        assert!(rl.check_at(t0, "register", "a", L3), "other bucket unaffected");
        assert!(rl.check_at(t0 + Duration::from_secs(60), "login", "a", L3), "window elapsed");
    }

    #[test]
    fn key_cap_fails_closed_then_recovers() {
        let rl = RateLimiter::new();
        let t0 = Instant::now();
        let one = Limit { max: 1, window: Duration::from_secs(60) };
        for i in 0..MAX_KEYS {
            assert!(rl.check_at(t0, "b", &i.to_string(), one));
        }
        assert!(!rl.check_at(t0, "b", "overflow", one), "table full of live entries");
        let later = t0 + Duration::from_secs(301);
        assert!(rl.check_at(later, "b", "overflow", one), "stale entries are reclaimed");
    }

    #[test]
    fn ip_keys() {
        let k = |s: &str| ip_key(s.parse().unwrap());
        assert_eq!(k("192.168.1.5"), "192.168.1.5");
        assert_eq!(k("::ffff:192.168.1.5"), "192.168.1.5");
        assert_eq!(k("2001:db8:1:2:aaaa:bbbb:cccc:dddd"), k("2001:db8:1:2::1"));
        assert_eq!(k("2001:db8:1:2::1"), "2001:db8:1:2::/64");
        assert_ne!(k("2001:db8:1:3::1"), k("2001:db8:1:2::1"));
    }
}
