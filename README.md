# FerrisCTF 🦀

A high-performance, self-contained Capture The Flag (CTF) platform built in **Rust** using **Axum**, **SQLx (SQLite)**, and **MiniJinja**.

Designed for resilience, security, and low operational complexity, FerrisCTF runs as a single compiled binary with zero external runtime dependencies and embedded template/static assets.

---

## 🚀 Features

- **Team Management & Onboarding**: Self-service registration, join codes (Crockford Base32), team creation, member management, captain transfer, and automated locking.
- **Dynamic Challenge Allocation**: Difficulty-based quota pooling per team (`Easy`, `Medium`, `Hard`) ensuring fair challenge distribution.
- **Secure Flag Verification**: Constant-time HMAC-based flag validation with salt to prevent timing attacks and flag leakage.
- **Scoring & Real-Time Scoreboard**: Dynamic score tracking, solve history, manual score adjustments, tie-breaking by solve timestamps, and cached JSON scoreboard endpoints.
- **Multi-Level Role Gate**:
  - **Player**: Solve challenges, request hints, view scoreboards.
  - **Team Captain**: Manage team roster, rename team, disband, transfer leadership.
  - **Admin**: View audit logs, adjust team scores, lock/unlock teams, export CSV data, ban/unban users.
  - **Ferris (Operator)**: Control rounds/events, create & toggle challenges, manage hints, promote/demote admins.
- **Hardened Security Stack**:
  - CSRF protection via double-submit pre-session cookies and token binding.
  - Custom sliding-window rate limiting per client IP.
  - Argon2id password hashing with multi-core concurrency bounds.
  - Path-based admin isolation (returns identical 404s for invalid routes and unauthorized callers).
  - Security headers: CSP, X-Frame-Options, X-Content-Type-Options, Referrer-Policy, Cache-Control.

---

## 🛠️ Tech Stack

- **Language & Runtime**: Rust (2021 Edition), Tokio async runtime
- **Web Framework**: Axum 0.7
- **Database**: SQLite via SQLx 0.7 (with automatic WAL mode and embedded migrations)
- **Templating & Assets**: MiniJinja 2.0 (HTML auto-escaping), `rust-embed` (embedded static assets & templates)
- **Cryptography**: Argon2id (`argon2`), HMAC-SHA256 (`hmac`, `sha2`), Constant-Time comparison (`subtle`)

---

## 📦 Prerequisites

Ensure you have the following installed:
- **Rust Toolchain** (1.75 or higher): Install via [rustup.rs](https://rustup.rs/)
- **SQLite 3**: Native SQLite library (`libsqlite3-dev` on Debian/Ubuntu, `sqlite` on Fedora/Arch)

Optional helper tool:
```bash
cargo install sqlx-cli --no-default-features --features sqlite
```

---

## ⚙️ Configuration

FerrisCTF is configured using environment variables or a `.env` file in the working directory.

Copy the provided template to create your `.env` file:
```bash
cp .env.example .env
```

### Environment Variables Reference

| Variable | Required | Default | Description |
|---|---|---|---|
| `SERVER_SECRET` | **Yes** | — | Minimum 32-character secret key used for HMAC subkeys (flags, sessions, CSRF). **Immutable once created.** |
| `ADMIN_USERNAME` | **Yes** | — | College ID for the bootstrap operator account created on first run. |
| `ADMIN_PASSWORD` | **Yes** | — | Password for the bootstrap operator account. |
| `DATABASE_URL` | **Yes** | `sqlite://ferrisctf.db?mode=rwc` | SQLite database connection URL. |
| `BIND_ADDR` | No | `0.0.0.0:8080` | Socket address for the HTTP server to listen on. |
| `COOKIE_SECURE` | No | `false` | Set to `true` if serving strictly over HTTPS. Must be `false` over plain HTTP. |
| `RUST_LOG` | No | `info,ferrisctf=debug` | Logging level for tracing output. |
| `POOL_SIZE_PER_TEAM` | No | `15` | Total challenge pool size assigned per team. |
| `PLANNED_QUESTIONS` | No | `60` | Global planned question limit. |
| `ROUND_COUNT` | No | `4` | Total number of contest rounds. |
| `REPEAT_PENALTY_POINTS` | No | `10` | Point penalty per failed flag submission attempt. |
| `GRACE_PERIOD_MINUTES` | No | `30` | Grace period duration (minutes) for team joins after event start. |
| `MAX_UPLOAD_BYTES` | No | `31457280` (30MB) | Max upload size limit for challenge ZIP archives. |
| `SESSION_TTL_HOURS` | No | `24` | Session expiration duration in hours. |
| `SCOREBOARD_CACHE_TTL_SECONDS` | No | `30` | In-memory cache duration for scoreboard queries. |

---

## 🏃 Running the Application

### 1. Database Setup
Database schema migrations run **automatically on application startup**. 

If you wish to run migrations manually via `sqlx-cli`:
```bash
sqlx migrate run
```

### 2. Development Mode
Run the application with live logging:
```bash
cargo run
```
The server will start on `http://localhost:8080` (or your configured `BIND_ADDR`).

### 3. Production Build
Build an optimized release binary:
```bash
cargo build --release
```
The compiled binary will be available at `./target/release/ferrisctf`.

Run the release binary:
```bash
./target/release/ferrisctf
```

---

## 🧪 Testing & Verification

Run the full test suite (55 unit & integration tests):
```bash
cargo test
```

Run Clippy code lints:
```bash
cargo clippy --all-targets --all-features
```

Run cargo check:
```bash
cargo check
```

---

## 📁 Directory Structure

```
├── Documentation/            # Technical specification, progress notes, and README
│   ├── FerrisCTF_Technical_Specification_v5.md
│   ├── progress.md
│   └── README.md
├── migrations/               # SQLite database migration scripts
│   └── 0001_init.sql
├── src/                      # Application Rust source code
│   ├── main.rs               # App entrypoint and background worker tasks
│   ├── app.rs                # Router assembly and middleware wiring
│   ├── config.rs             # Configuration loading & validation
│   ├── db.rs                 # Database pool initialization & WAL mode setup
│   ├── errors.rs             # Application error types & HTTP mappings
│   ├── keys.rs               # HMAC subkey derivation
│   ├── csrf.rs               # Double-submit CSRF token validation
│   ├── ratelimit.rs          # Sliding-window IP rate limiter
│   ├── session.rs            # Database-backed session store
│   ├── state.rs              # Shared AppState container
│   ├── templates.rs          # Embedded template engine & renderer
│   ├── middleware/           # Axum middleware (auth, admin guard, csrf, security headers)
│   ├── models/               # Domain structs (User, Team, Challenge, Round, etc.)
│   ├── routes/               # Route handlers (auth, onboarding, team, challenges, admin, etc.)
│   └── services/             # Core business logic (scoring, pool, verifier, rounds, team)
├── static/                   # Static CSS & assets (embedded at build time)
│   └── style.css
└── templates/                # MiniJinja HTML templates (embedded at build time)
    ├── base.html
    ├── auth/
    ├── onboarding/
    ├── team/
    ├── challenges/
    ├── admin/
    └── errors/
```

---

## 📄 License

Internal / Proprietary contest software developed for FerrisCTF.
