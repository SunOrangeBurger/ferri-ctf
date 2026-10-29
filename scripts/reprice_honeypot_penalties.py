#!/usr/bin/env python3
"""Reprice recorded honeypot penalties from 50% to 20%.

This preserves the original adjustment IDs (which prevent duplicate penalties),
replays each affected team's scoring events in time order, and appends an audit
entry for every revised adjustment. Run once against the event database.
"""
import argparse
import json
import sqlite3
import uuid
from pathlib import Path


def twenty_percent_delta(score: int) -> int:
    return score * 4 // 5 - score if score > 0 else 0


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", default="ferrisctf.db")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    db = sqlite3.connect(Path(args.database))
    db.row_factory = sqlite3.Row
    try:
        db.execute("BEGIN IMMEDIATE")
        operator = db.execute(
            "SELECT id FROM users WHERE is_ferris=1 ORDER BY created_at,id LIMIT 1"
        ).fetchone()
        if not operator:
            raise RuntimeError("no Ferris operator account is available for audit records")

        penalties = db.execute(
            "SELECT id,team_id,delta,reason,created_at FROM score_adjustments "
            "WHERE id LIKE 'honeypot:%' ORDER BY team_id,created_at,id"
        ).fetchall()
        by_team = {}
        for row in penalties:
            by_team.setdefault(row["team_id"], []).append(row["id"])

        revised = []
        for team_id in by_team:
            events = db.execute(
                "SELECT solved_at AS at, 0 AS event_order, challenge_id AS row_id, "
                "points_awarded AS delta, 'solve' AS kind FROM solves WHERE team_id=? "
                "UNION ALL "
                "SELECT created_at AS at, 1 AS event_order, id AS row_id, delta, 'adjustment' "
                "FROM score_adjustments WHERE team_id=? "
                "ORDER BY at,event_order,row_id",
                (team_id, team_id),
            ).fetchall()
            score = 0
            for event in events:
                if event["kind"] != "adjustment" or not event["row_id"].startswith("honeypot:"):
                    score += event["delta"]
                    continue
                old_delta = event["delta"]
                new_delta = twenty_percent_delta(score)
                if old_delta != new_delta:
                    revised.append((event["row_id"], team_id, old_delta, new_delta, score))
                score += new_delta

        if args.dry_run:
            db.rollback()
        else:
            for adjustment_id, team_id, old_delta, new_delta, score_before in revised:
                challenge_id = adjustment_id.rsplit(":", 1)[1]
                before = score_before
                after = before + new_delta
                reason = (
                    f"Honeypot token submitted; challenge {challenge_id}; "
                    f"20% score decrement: {before} -> {after}. "
                    "Token use is not proof of AI use."
                )
                db.execute(
                    "UPDATE score_adjustments SET delta=?,reason=? WHERE id=?",
                    (new_delta, reason, adjustment_id),
                )
                details = json.dumps({
                    "adjustment_id": adjustment_id,
                    "old_delta": old_delta,
                    "new_delta": new_delta,
                    "score_before": before,
                    "score_after": after,
                    "penalty_percent": 20,
                    "automated": True,
                }, separators=(",", ":"))
                db.execute(
                    "INSERT INTO audit_log(id,admin_id,action,target_type,target_id,details) "
                    "VALUES(?,?,'honeypot_penalty_repriced','team',?,?)",
                    (str(uuid.uuid4()), operator["id"], team_id, details),
                )
            db.commit()

        old_total = sum(-old for _, _, old, _, _ in revised)
        new_total = sum(-new for _, _, _, new, _ in revised)
        print(f"{'Would reprice' if args.dry_run else 'Repriced'} {len(revised)} penalties "
              f"across {len(by_team)} teams: {old_total} deducted -> {new_total} deducted.")
    except BaseException:
        db.rollback()
        raise
    finally:
        db.close()


if __name__ == '__main__':
    main()
