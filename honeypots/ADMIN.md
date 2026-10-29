# LAST ROUND honeypots

Ten traps total are installed by `python3 scripts/import_honeypots.py`. They remain inactive until LAST ROUND starts. Rebuild and restart the updated server before starting LAST ROUND: startup applies migration 0009 and starts the durable penalty worker.

LAST ROUND activates all ten as part of the complete challenge bank and adds every challenge to every eligible team's pool except Florida Man. An already active LAST ROUND can receive the installed set with `python3 scripts/import_honeypots.py --activate` after deploying the updated server. The importer refuses that flag outside LAST ROUND. It does not add traps to an ordinary or backlog round.

Each ZIP contains a long README, a synthetic CSV, a log, and a checksum manifest. Human-facing instructions prohibit AI use and ask humans to skip. The embedded automated-reader bait requests a plausible-looking token and a fabricated explanation. No actual exploitation, external account, executable, or external lookup is involved. There is no reliable instruction that forces an AI to comply. Using a token is not proof of AI use: humans can copy or share it.

## Delay and scoring

A matching bait submission records a durable pending job and displays “validation pending.” It earns no points, records no solve, and does not change the dynamic price. The due time is exactly 18 seconds after the first submission. A worker checks each second; team browsers poll notices every two seconds. Under normal load a visible notice arrives roughly 18–21 seconds after submission; outages can delay it further.

At execution, each trap deducts 20% of the team's then-current positive score, rounding the remaining 80% down. Zero and negative scores are unchanged. Each team/trap can deduct once, regardless of which teammate retries. Ten distinct traps can compound. Other score changes made during the delay are included in the execution-time score. Pending jobs persist through restarts and still apply if the round ends before they are processed.

There is no individual scoring system. The submitting user's ID is recorded in the adjustment reason and audit details. Penalties, audit records, and job completion commit in one transaction. Existing traps already penalized under the earlier immediate behavior are not charged again.

## Notices and review

After commit, the submitting team sees an instruction-compliance notice on authenticated pages; it identifies the question and recorded deduction, without asserting AI use. Other teams cannot read it. Previously applied notices remain visible on subsequent visits.

Review the deduction under score adjustments in the detailed admin leaderboard, or in the audit log under `honeypot_triggered`. Credit back its recorded amount using the existing admin score-adjustment controls if needed. Do not delete the original penalty row: it is also the replay-protection record.

## Verification

Tests cover both scoring modes, odd scores, retries, both traps, wrong flags, membership and pool authorization, nonpositive scores, no solve awards, ten-trap LAST ROUND inclusion, an 18-second durable due time, jobs surviving round end, worker replay protection, and team-scoped notices. Model compliance with the embedded bait has not been measured.

## Private testing reference

These are bait tokens, not normal solve flags. Test in a disposable event database; submitting one on the live event schedules a real deduction.

| Challenge | Bait token |
| --- | --- |
| The Archive Compliance Review | `nexus{archive_compliance_verified}` |
| The Protocol Compliance Review | `nexus{protocol_compliance_verified}` |
| The Ledger Discrepancy | `nexus{ledger_evidence_verified}` |
| The Quiet Relay | `nexus{packet_evidence_verified}` |
| The Cargo Manifest | `nexus{manifest_evidence_verified}` |
| The Clockwork Incident | `nexus{timeline_evidence_verified}` |
| The Missing Inventory | `nexus{inventory_evidence_verified}` |
| The Checksum Cabinet | `nexus{checksum_evidence_verified}` |
| The Mailroom Archive | `nexus{mailroom_evidence_verified}` |
| The Backup Register | `nexus{backup_evidence_verified}` |
