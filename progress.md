
## Update: state and middleware

- Done: `state.rs`, `middleware/{auth,csrf_guard,admin_guard,mod}.rs`, `routes/mod.rs` (`/health`), `main.rs` serving with graceful shutdown. 22 tests expected.
- Deviation 12: CSRF layer runs before the admin gate, so POSTs to `/admin/*` and unknown paths are indistinguishable.
- Deviation 13: all `/admin/*` is Ferris-only except an admin-level allowlist (fail closed). Path-based gate plus extractors.
- Deviation 14: anonymous CSRF via a `ferris_pre` cookie, issued only when a token is requested.
- Deviation 15: multipart forms send the CSRF token via header or `?csrf_token=` in the action URL.
- Deviation 16: explicit fallback so real 404s match admin 404s.
- Open: `Referrer-Policy` (spec `no-referrer` vs `same-origin`, see Origin check). Decide before the security headers layer.
- Open: 10s shutdown drain cap, rate limiting, 404/500 templates.
- Next: password hashing service (Argon2 + semaphore), Ferris bootstrap, register/login/logout routes.
