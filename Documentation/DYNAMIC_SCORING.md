# Live dynamic challenge scoring

Dynamic scoring is optional and disabled by default. Static contests continue to
use each challenge's saved `points`; existing solve awards and ranking rules are
not migrated or recalculated.

## Operator setup

1. Sign in as the **Ferris/operator**, open **Admin → Scoring Controls**, and
   select Dynamic before starting the first round. Ordinary admins and players
   cannot access this page or its update endpoints.
2. Review the three difficulty rules and save any changes with a reason.
3. Start rounds using the existing event controls. Scoring history carries across
   rounds. Neither pool size nor challenge allocation changes.
4. During the event, return to Scoring Controls to edit rules or override an
   individual challenge's current points. Every change requires a reason and is
   written to the existing audit log with its operator and before/after values.

The scoring mode cannot change once a round/event has started. **Dynamic rules
and individual values remain editable at any time**, including between rounds.
To avoid overwriting another settings edit, reload a stale settings page before
saving again. All scoring controls use server-side Ferris authorization, signed
sessions, existing CSRF/origin checks, and `Cache-Control: no-store`.

## Defaults and calculation

| Difficulty | Starting | Minimum | Maximum | Quiet minutes | Recovery/minute | Solve drop |
|---|---:|---:|---:|---:|---:|---:|
| Easy | 100 | 40 | 150 | 15 | 1 | 15% |
| Medium | 300 | 175 | 400 | 20 | 2 | 10% |
| Hard | 700 | 450 | 1,000 | 30 | 3 | 5% |

There is one global price per challenge. A correct, first solve by a team earns
the current price rounded down, less that team's existing hint/repeat deductions
(minimum award zero). That award is permanently recorded in `solves`.

After awarding the solve, the price becomes:

```text
minimum + (price_before_solve - minimum) * (1 - drop_percent / 100)
```

With no recovery between solves, awards to teams 1, 10 and 50 are respectively:

| Difficulty | Team 1 | Team 10 | Team 50 |
|---|---:|---:|---:|
| Easy | 100 | 53 | 40 |
| Medium | 300 | 223 | 175 |
| Hard | 700 | 607 | 470 |

After the quiet period, the price increases linearly. Each distinct successful
team solve restarts the quiet period, even when deductions make its award zero.
Incorrect flags and repeat submissions have no pricing effect. After a solve,
recovery cannot exceed the previous solver's **gross challenge value**, before
hint/repeat deductions. Before the first solve, the ceiling is the configured
maximum. Consequently, different deductions can still produce different net
awards even when gross values match.

Recovery counts only time when the challenge is active and is assigned to at
least one locked, non-disbanded team in an active round. The timer starts on its
first eligible pool assignment, pauses during round gaps/deactivation, and is
capped at a round's scheduled deadline even if the auto-end task runs late.
Already elapsed availability and solve state survive process restarts. Server
downtime counts if the round remains scheduled as active in the database.

Prices use integer millionths of a point internally, avoiding floating-point
drift. Values are computed on demand; no background price-update job is needed.

## Editing during an event

- Rule edits checkpoint recovery at the old rate before applying the new rule.
  They retain the actual last-solve clock; changing the quiet period does not
  retroactively award additional recovery. New bounds clamp current values and
  stored ceilings immediately.
- Starting points apply only to challenges whose price has not been initialized.
  A challenge initializes on first availability or an explicit manual override.
- A manual override accepts any integer within the difficulty's bounds. It
  restarts the quiet period and clears the previous-solver ceiling, allowing
  recovery to the configured maximum until the next solve.
- Neither operation changes points already earned. Administrator overrides or
  raised minimums can deliberately raise future awards above earlier ones; the
  reason and before/after values are audited.
- Static challenge points remain stored separately. In dynamic mode, use Scoring
  Controls for current values; the challenge bank labels its old field as
  **Static points**.

Bounds must satisfy `0 <= minimum <= starting <= maximum <= 1,000,000`.
Recovery is an integer from 1 to 1,000,000 points/minute, the quiet period is 1 to
10,080 minutes, and the drop is an integer from 1 to 99 percent.

## Player display and endpoints

Challenge list/detail pages show the current effective award for unsolved
challenges, and fixed earned points for solved ones (including zero). Dynamic
pages refresh their price text every ten seconds without replacing flag inputs.
Without JavaScript, a page refresh shows a fresh quote. The server's value at
submission acceptance determines the award, so another team's solve can change
it after a player loads the page.

| Endpoint | Access |
|---|---|
| `GET /admin/scoring` | Ferris only |
| `POST /admin/scoring` | Ferris + CSRF; mode/rule edit |
| `POST /admin/scoring/challenges/:id/points` | Ferris + CSRF; override |
| `GET /api/challenges/points` | Signed-in team; only its eligible active pool |

The quote endpoint returns challenge IDs, gross/effective integer points,
the caller's earned points, dynamic-mode status, and a server timestamp. It
does not expose administrative rules, other pools, flag data, or audit reasons.
Every successful solve and pricing mutation takes the SQLite writer lock before
reading state. Acceptance rechecks team membership, pool access, round deadline,
visibility, and prior solves inside that transaction.

## Validation and limitations

Run `cargo test --locked`, `cargo check --locked`, and
`cargo clippy --locked --all-targets --all-features`.
For a disposable HTTP stress test of both roles, concurrent solves/edits,
round transitions, and restart persistence, see
[Stress testing](STRESS_TESTING.md) and `scripts/stress_scoring.py`.
Tests cover the legacy static path, bounds/rounding, quiet periods, live edits,
restart persistence, visibility/round pauses, 50 concurrent teams, duplicate
solves, authorization/CSRF, and a deterministic six-hour model with 30 and 45
challenges and up to 50 teams per challenge. The model is not a production load
benchmark.

This mechanism reduces rewards for rapid sharing of a common flag; it cannot
detect or prevent flag sharing. Any recovering price can make delayed submission
attractive. The previous-solver ceiling limits that advantage, and the default
hard minimum stays above the default medium maximum, before deductions.
Randomized pools can still expose teams to different challenges and prices. The
leaderboard ranks awarded points descending, then the time the current score was
reached ascending. Equal points and times share a rank. Performance and completion
ratios are informational and do not override this ordering. Opportunity values
remain frozen when assigned; dynamic assignments use the configured difficulty
maximum as that ceiling.

The migration is additive and seeds static mode on existing databases. Select
dynamic mode on a fresh event database before round one; a running static event
keeps its existing behavior. Back up the SQLite database before deployment as
with other platform migrations.
