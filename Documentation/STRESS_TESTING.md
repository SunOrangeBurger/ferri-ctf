# HTTP stress test and regression results

Verified on 15 October 2026 using the optimized release binary on a local
macOS x86_64 machine (16 logical CPUs), Rust 1.99.0, Python 3.11.9 and httpx 0.28.1.
The runner creates its own loopback server, database and synthetic accounts.

## Final results

Each profile uses **50 teams, 100 participant accounts and 45 assigned challenges**,
plus one initially out-of-pool challenge for access-denial checks. Both team
members can submit concurrently. Measurements include real HTTP middleware,
signed sessions, CSRF validation, SQLite, templates and JSON serialization.

| Profile | Recorded requests | Total duration | Price quote P95 | Solve P95 | Result |
|---|---:|---:|---:|---:|---|
| Dynamic, 100 concurrent | 3,173 | 25.15 s | 2505.83 ms | 751.30 ms | Pass |
| Static, 100 concurrent | 3,112 | 19.46 s | 1461.22 ms | 659.64 ms | Pass |
| Dynamic burst, 200 concurrent | 6,173 | 51.25 s | 3936.82 ms | 731.22 ms | Pass |
| Dynamic steady, 20 reads/second | 2,373 | 68.15 s | 5.41 ms | 707.51 ms | Pass |

Across the four final profiles: **14,831 recorded requests, zero 5xx responses,
zero unexpected responses and all correctness checks passed**. Expected 4xx
responses cover authorization, CSRF, validation and enforced rate limits.
The paced profile sends 1,200 mixed reads over one minute, then runs the same
concurrent writes and lifecycle checks. Unbounded burst profiles deliberately
saturate both the local server and load generator; their latency is higher.

These are local loopback results, without deployment hardware, TLS or WAN
latency. They are not a six-hour real-time soak. The existing deterministic
six-hour scoring simulation remains part of the passing Rust suite.

## Issues reproduced and corrected

- **Hint/solve race:** simultaneous requests could unlock a hint after the solve
  without charging its cost. Reproduced in a Rust concurrency test and an HTTP
  run. Hint eligibility, unlock and solve acceptance now share the SQLite writer
  ordering. A hint is either unlocked before the solve and deducted, or rejected
  after the solve. Inactive challenge hints are also rejected.
- **Repeated pricing queries:** live quotes fetched state and clocks separately
  for every challenge. Pricing now batches those reads; challenge cards also
  fetch hint costs with the pool query.
- **Leaderboard refresh contention:** concurrent cache misses could rebuild the
  same leaderboard and publish stale data after invalidation. Refreshes are now
  serialized through publication. Team members, history and totals are batched
  into one database snapshot, preserving ranking and adjustment semantics.

## Coverage

- Each profile records 350 unique solves, duplicate submissions from both team
  members, incorrect and forbidden flags, hint/solve interleavings, and the
  unchanged per-team submission rate limit.
- Dynamic profiles interleave price polling and solves with 20 rule edits and
  20 current-value overrides, then verify the final values, audit entries,
  exponential awards and unchanged historical scores.
- Guest, participant and ordinary-admin requests cannot read or mutate scoring
  controls. Tests include direct endpoints, forged sessions, invalid CSRF,
  cross-session CSRF, cross-origin POSTs, invalid values and out-of-pool flags.
- Real player/operator logins succeed. Quote responses exclude secret/admin
  data. Banned users lose access on their next request.
- Ending a round clears available quotes and prevents solving; the next round
  assigns 15 challenges per team and excludes solved challenges. Server restart
  preserves prices and awards. SQLite integrity and foreign-key checks pass.
- Browser verification: a participant received 404 for the admin URL; a quote
  changed from 300 to 350 without clearing their flag draft; submission earned
  350; an operator then changed the current value to 400 while the earned award
  stayed 350. Live operator rule editing and browser console checks passed.

**73 Rust unit/integration tests pass**, including new regression cases for
hint/solve ordering, inactive hints, batched quote parity, concurrent leaderboard
invalidation and score-adjustment history. `cargo check --locked` and
`cargo clippy --locked --all-targets --all-features` also pass.

## Reproduce

The optional runner requires Python and httpx; production dependencies are
unchanged. Run it from the repository root:

```bash
cargo build --locked --release
python3 -m venv /tmp/ferri-stress-venv
/tmp/ferri-stress-venv/bin/pip install httpx==0.28.1
/tmp/ferri-stress-venv/bin/python scripts/stress_scoring.py --output /tmp/ferri-dynamic100 --reads 2000 --concurrency 100
/tmp/ferri-stress-venv/bin/python scripts/stress_scoring.py --output /tmp/ferri-static100 --reads 2000 --concurrency 100 --static
/tmp/ferri-stress-venv/bin/python scripts/stress_scoring.py --output /tmp/ferri-burst200 --reads 5000 --concurrency 200
/tmp/ferri-stress-venv/bin/python scripts/stress_scoring.py --output /tmp/ferri-steady20 --reads 1200 --concurrency 100 --rps 20
```

Choose a fresh output directory for each run. The script refuses to overwrite
an existing directory and always starts its own local server; it cannot target
an existing contest URL. It exits nonzero on an unexpected response or failed
check. `report.json` contains per-endpoint P50/P95/P99/max, status counts,
correctness checks and the tested binary SHA-256. The output also contains
synthetic fixtures and private test credentials; publish the report rather than
the entire output directory.
