#!/usr/bin/env python3
"""Disposable localhost HTTP stress/regression test. Requires Python + httpx.

Build first: cargo build --locked --release
Run: python3 scripts/stress_scoring.py --output /tmp/ferri-stress
Never points at a deployed contest: this script owns its server and database.
"""
import argparse
import asyncio
import base64
import collections
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import socket
import sqlite3
import subprocess
import tempfile
import time

import httpx


SCALE = 1_000_000


class Stress:
    def __init__(self, args):
        self.args = args
        self.output = Path(args.output).resolve()
        self.output.mkdir(parents=True, exist_ok=False)
        self.db_path = self.output / "fixture.db"
        self.secret = secrets.token_hex(32).encode()
        self.password = secrets.token_urlsafe(18)
        self.keys = {name: hmac.digest(self.secret, f"ferrisctf/v1/{name}".encode(), "sha256")
                     for name in ["flag", "session", "csrf"]}
        self.identities = {}
        self.metrics = collections.defaultdict(list)
        self.codes = collections.Counter()
        self.failures = []
        self.checks = []
        self.sem = asyncio.Semaphore(args.concurrency)
        self.process = None
        self.log = None
        self.binary_hash = hashlib.sha256(Path(args.binary).read_bytes()).hexdigest()
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            self.port = sock.getsockname()[1]
        self.base = f"http://127.0.0.1:{self.port}"
        self.environment = dict(os.environ, SERVER_SECRET=self.secret.decode(),
            ADMIN_USERNAME="stress_operator", ADMIN_PASSWORD=self.password,
            DATABASE_URL=f"sqlite://{self.db_path}?mode=rwc", BIND_ADDR=f"127.0.0.1:{self.port}",
            RUST_LOG="warn")

    def connect(self):
        db = sqlite3.connect(self.db_path, timeout=30)
        db.execute("PRAGMA foreign_keys=ON")
        return db

    def identity(self, db, user):
        token = base64.urlsafe_b64encode(secrets.token_bytes(32)).decode().rstrip("=")
        session_id = hashlib.sha256(token.encode()).hexdigest()
        signature = hmac.new(self.keys["session"], token.encode(), "sha256").hexdigest()
        nonce = secrets.token_hex(16)
        csrf = nonce + "." + hmac.new(self.keys["csrf"], f"{session_id}:{nonce}".encode(), "sha256").hexdigest()
        db.execute("INSERT INTO sessions(id_hash,user_id,expires_at) VALUES(?,?,strftime('%Y-%m-%d %H:%M:%f','now','+24 hours'))", (session_id, user))
        self.identities[user] = {"cookie": f"ferris_session={token}.{signature}", "csrf": csrf}

    def seed(self):
        with self.connect() as db:
            operator, password_hash = db.execute("SELECT id,password_hash FROM users WHERE username='stress_operator'").fetchone()
            self.identity(db, operator)
            self.operator = operator
            db.execute("UPDATE scoring_settings SET enabled=?", (not self.args.static,))
            db.execute("INSERT INTO users(id,username,display_name,password_hash,is_admin) VALUES('admin','stress_admin','Ordinary Admin',?,1)", (password_hash,))
            self.identity(db, "admin")
            for i in range(100):
                user = f"user{i:03}"
                db.execute("INSERT INTO users(id,username,display_name,password_hash) VALUES(?,?,?,?)", (user, user, f"Player {i}", password_hash))
                self.identity(db, user)
            for i in range(50):
                team = f"team{i:02}"
                db.execute("INSERT INTO teams(id,name,join_code,captain_id,is_locked) VALUES(?,?,?,?,1)", (team, f"Stress Team {i:02}", f"TEST{i:04}", f"user{2*i:03}"))
                for member in range(2):
                    db.execute("INSERT INTO team_members(team_id,user_id,role) VALUES(?,?,?)", (team, f"user{2*i+member:03}", "captain" if member == 0 else "member"))
            for i in range(46):
                difficulty = ["easy", "medium", "hard"][i % 3]
                flag = f"ferris{{stress_{i:02}}}"
                salt = secrets.token_hex(16)
                mac = hmac.new(self.keys["flag"], (salt + flag).encode(), "sha256").hexdigest()
                db.execute("INSERT INTO challenges(id,title,description,category,difficulty,points,flag_hmac,flag_salt) VALUES(?,?,?,?,?,?,?,?)", (f"c{i:02}", f"Stress Challenge {i:02}", "Disposable stress test challenge", "Web", difficulty, [100, 300, 700][i % 3], mac, salt))
            db.execute("UPDATE event_state SET started_at=strftime('%Y-%m-%d %H:%M:%f','now')")
            db.execute("INSERT INTO rounds(round_number,started_at,auto_end_at) VALUES(1,strftime('%Y-%m-%d %H:%M:%f','now'),strftime('%Y-%m-%d %H:%M:%f','now','+6 hours'))")
            for i in range(50):
                for c in range(45):
                    db.execute("INSERT INTO team_challenge_pool(team_id,round_number,challenge_id) VALUES(?,1,?)", (f"team{i:02}", f"c{c:02}"))
            db.execute("INSERT INTO hints(id,challenge_id,content,point_cost) VALUES('hint06','c06','A useful hint',10)")
        # Test-only credentials stay outside the repository, in a private file.
        credentials = self.output / "preview.json"
        credentials.write_text(json.dumps({"base_url": self.base, "password": self.password,
            "server_secret": self.secret.decode(), "operator": "stress_operator", "player": "user000"}))
        credentials.chmod(0o600)

    async def start(self):
        self.log = (self.output / "server.log").open("a")
        self.process = subprocess.Popen([str(Path(self.args.binary).resolve())], env=self.environment,
            stdout=self.log, stderr=subprocess.STDOUT)
        async with httpx.AsyncClient(timeout=2, trust_env=False) as client:
            for _ in range(100):
                if self.process.poll() is not None:
                    raise RuntimeError("Server exited: " + (self.output / "server.log").read_text()[-1000:])
                try:
                    if (await client.get(self.base + "/health")).status_code == 200:
                        return
                except httpx.HTTPError:
                    pass
                await asyncio.sleep(0.1)
        raise RuntimeError("Server did not become healthy")

    def stop(self):
        if self.process is not None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
            self.process = None
        if self.log:
            self.log.close()
            self.log = None

    async def request(self, client, label, path, user=None, data=None, expected=(200,), csrf=True, extra=None):
        headers = {}
        if user:
            headers["cookie"] = self.identities[user]["cookie"]
            if data is not None and csrf:
                headers["x-csrf-token"] = self.identities[user]["csrf"]
        if extra:
            headers.update(extra)
        async with self.sem:
            start = time.perf_counter()
            try:
                response = await client.request("POST" if data is not None else "GET", self.base + path,
                    headers=headers, data=data)
                self.metrics[label].append((time.perf_counter() - start) * 1000)
                self.codes[str(response.status_code)] += 1
                if response.status_code not in expected:
                    self.failures.append({"label": label, "path": path, "status": response.status_code, "body": response.text[:180]})
                return response
            except httpx.HTTPError as error:
                self.metrics[label].append((time.perf_counter() - start) * 1000)
                self.codes["transport_error"] += 1
                self.failures.append({"label": label, "path": path, "error": str(error)})

    def check(self, name, condition, detail=None):
        self.checks.append({"name": name, "passed": bool(condition), "detail": detail})

    async def security(self, client):
        for user in [None, "user000", "admin"]:
            for path in ["/admin/scoring", "/admin/scoring/challenges/c00/points"]:
                await self.request(client, "access_denied", path, user, expected=(404,))
                await self.request(client, "access_denied", path, user,
                    data={"points": 150, "reason": "unauthorized", "action": "mode", "enabled": 0, "revision": 0},
                    expected=(404,) if user else (403,))
        for path in ["/admin/scoring", "/admin/scoring/challenges/c00/points"]:
            for extra in [{}, {"x-csrf-token": "forged"}, {"x-csrf-token": self.identities["user000"]["csrf"]}, {"origin": "https://attacker.invalid"}]:
                await self.request(client, "csrf_denied", path, self.operator,
                    data={"points": 140, "reason": "csrf rejection"}, csrf=False, extra=extra, expected=(403,))
        for points in ["-1", "1000000000000", "hello"]:
            await self.request(client, "validation", "/admin/scoring/challenges/c00/points", self.operator,
                data={"points": points, "reason": "invalid"}, expected=(400, 409, 422))
        for path in ["/challenges/c45", "/challenges/c45/submit"]:
            await self.request(client, "pool_denied", path, "user000", data={"flag": "ferris{stress_45}"} if path.endswith("submit") else None, expected=(404,))
        response = await self.request(client, "quote_scope", "/api/challenges/points", "user000")
        if response is not None and response.status_code == 200:
            payload = response.json()
            self.check("quote response stays inside assigned pool", len(payload["challenges"]) == 45 and all(c["id"] != "c45" for c in payload["challenges"]))
            self.check("quote response excludes secrets/settings", not any(s in response.text for s in ["flag_hmac", "drop_percent", "flag_salt", "reason"]))
        await self.request(client, "unauthenticated", "/api/challenges/points", expected=(303,))
        await self.request(client, "forged_session", "/admin/scoring", extra={"cookie": "ferris_session=forged.signature"}, expected=(404,))
        wrong = await self.request(client, "incorrect_flag", "/challenges/c00/submit", "user000",
            data={"flag": "incorrect"}, expected=(303,))
        self.check("wrong flag rejected without solve", wrong is not None and "status=incorrect" in wrong.headers.get("location", ""))
        with self.connect() as db:
            self.check("wrong/forbidden flags do not alter prices or add solves", not db.execute("SELECT 1 FROM solves").fetchall()
                and (self.args.static or db.execute("SELECT anchor_units FROM challenge_pricing WHERE challenge_id='c00'").fetchone()[0] == 100*SCALE))

    async def login_flows(self):
        for path, username in [("/login", "user000"), ("/admin/login", "stress_operator")]:
            async with httpx.AsyncClient(timeout=10, follow_redirects=False, trust_env=False) as browser:
                page = await browser.get(self.base + path)
                token = re.search(r'name="csrf_token" value="([^\"]+)"', page.text).group(1)
                response = await browser.post(self.base + path,
                    data={"username": username, "password": self.password, "csrf_token": token})
                self.check("real login flow " + username, response.status_code == 303 and "ferris_session" in browser.cookies)
                if username == "user000":
                    blocked = await browser.get(self.base + "/admin/scoring")
                    self.check("browser participant cannot open admin controls", blocked.status_code == 404)
                else:
                    controls = await browser.get(self.base + "/admin/scoring")
                    self.check("browser operator can open live controls", controls.status_code == 200)

    async def reads(self, client):
        paths = ["/api/challenges/points"] * 4 + ["/challenges", "/challenges/c00", "/api/scoreboard", "/scoreboard", "/team", "/admin/scoring"]
        started = time.perf_counter()
        async def work(i):
            path = paths[i % len(paths)]
            return await self.request(client, "read:" + path, path,
                self.operator if path == "/admin/scoring" else f"user{i % 100:03}")
        if self.args.rps:
            tasks = []
            for i in range(self.args.reads):
                await asyncio.sleep(max(0, started + i / self.args.rps - time.perf_counter()))
                tasks.append(asyncio.create_task(work(i)))
            await asyncio.gather(*tasks)
        else:
            for offset in range(0, self.args.reads, 500):
                await asyncio.gather(*(work(i) for i in range(offset, min(offset + 500, self.args.reads))))
                print(f"Read load: {min(offset + 500, self.args.reads)}/{self.args.reads}", flush=True)
        return self.args.reads / (time.perf_counter() - started)

    async def live_edits(self, client):
        if self.args.static:
            return
        for i in range(20):
            await self.request(client, "admin_override", "/admin/scoring/challenges/c44/points", self.operator,
                data={"points": 750 + i, "reason": f"Concurrent stress override {i}"}, expected=(303,))
            page = await self.request(client, "admin_edit_page", "/admin/scoring", self.operator)
            if page is None or page.status_code != 200:
                continue
            revision = re.search(r'name="revision" value="(\d+)"', page.text).group(1)
            await self.request(client, "admin_rule", "/admin/scoring", self.operator,
                data={"action": "rule", "revision": revision, "reason": f"Concurrent stress rule {i}",
                    "difficulty": "hard", "starting_points": 700, "minimum_points": 450, "maximum_points": 1000,
                    "quiet_minutes": 30, "recovery_per_minute": 3 + i % 2, "drop_percent": 5}, expected=(303,))

    async def solving(self, client):
        async def wave(challenge):
            await asyncio.gather(*(self.request(client, "solve", f"/challenges/c{challenge:02}/submit", f"user{i*2:03}",
                data={"flag": f"ferris{{stress_{challenge:02}}}"}, expected=(303,)) for i in range(50)))
        async def polls():
            for _ in range(10):
                await asyncio.gather(*(self.request(client, "poll_during_writes", "/api/challenges/points", f"user{i*2:03}") for i in range(50)),
                    *(self.request(client, "scoreboard_during_writes", "/api/scoreboard", f"user{i*2:03}") for i in range(5)))
        async def solves():
            for challenge in range(6):
                await wave(challenge)
            await asyncio.gather(*(self.request(client, "duplicate", "/challenges/c00/submit", f"user{i:03}",
                data={"flag": "ferris{stress_00}"}, expected=(303,)) for i in range(100)))
        await asyncio.gather(solves(), polls(), self.live_edits(client))
        # Hint/solve interleavings must either charge the unlocked hint or reject
        # the hint request after a solve. A successful free late unlock is a bug.
        tasks = []
        for i in range(50):
            tasks.extend([self.request(client, "hint_race", "/challenges/c06/hint", f"user{i*2:03}",
                data={"hint_id": "hint06"}, expected=(303, 403)),
                self.request(client, "hint_race_solve", "/challenges/c06/submit", f"user{i*2+1:03}",
                    data={"flag": "ferris{stress_06}"}, expected=(303,))])
        await asyncio.gather(*tasks)
        with self.connect() as db:
            rows = db.execute("SELECT s.team_id,s.challenge_id,s.points_awarded FROM solves s ORDER BY s.rowid").fetchall()
            self.check("one solve per team/challenge", len(rows) == 350 and len({(r[0],r[1]) for r in rows}) == 350, len(rows))
            for challenge in ["c00", "c06"]:
                value = 100*SCALE
                incorrect = []
                for team, _, points in [r for r in rows if r[1] == challenge]:
                    hint = db.execute("SELECT EXISTS(SELECT 1 FROM hint_usage WHERE team_id=? AND hint_id='hint06')", (team,)).fetchone()[0] if challenge == "c06" else 0
                    expected = value // SCALE - 10*hint
                    if points != expected:
                        incorrect.append({"team": team, "expected": expected, "actual": points, "hint_unlocked": hint})
                    if not self.args.static:
                        value = 40*SCALE + (value - 40*SCALE)*85//100
                self.check("atomic scoring " + challenge, not incorrect, incorrect[:10])
            before = list(rows)
        # Wait for all writes to finish, then compare the actual displayed scoreboard.
        board = await self.request(client, "final_scoreboard", "/api/scoreboard", "user000")
        if board is not None and board.status_code == 200:
            with self.connect() as db:
                totals = dict(db.execute("SELECT team_id,SUM(points_awarded) FROM solves GROUP BY team_id"))
            wrong = [e["team_id"] for e in board.json()["scoreboard"] if e["total_score"] != totals[e["team_id"]]]
            self.check("scoreboard reflects all completed solves", not wrong, wrong)
        if not self.args.static:
            await self.request(client, "post_solve_override", "/admin/scoring/challenges/c00/points", self.operator,
                data={"points": 140, "reason": "Check earned awards remain fixed"}, expected=(303,))
        with self.connect() as db:
            after = db.execute("SELECT s.team_id,s.challenge_id,s.points_awarded FROM solves s ORDER BY s.rowid").fetchall()
            self.check("operator changes never rewrite earned awards", before == after)
            if not self.args.static:
                self.check("all live edits are audited", db.execute("SELECT COUNT(*) FROM audit_log WHERE target_type='scoring'").fetchone()[0] == 41)
                self.check("operator's final live values are saved", db.execute("SELECT anchor_units FROM challenge_pricing WHERE challenge_id='c44'").fetchone()[0] == 769*SCALE
                    and db.execute("SELECT recovery_per_minute FROM difficulty_scoring_rules WHERE difficulty='hard'").fetchone()[0] == 4)
        limited = await asyncio.gather(*(self.request(client, "submission_rate_limit", "/challenges/c07/submit", "user096",
            data={"flag": "wrong"}, expected=(303, 429)) for _ in range(25)))
        self.check("submission throttling stays enabled under load", any(r is not None and r.status_code == 429 for r in limited))

    async def integrity(self, client):
        await self.request(client, "end_round", "/admin/rounds/end", self.operator, data={"action": "end", "round_number": 1}, expected=(303,))
        paused = await self.request(client, "paused_quotes", "/api/challenges/points", "user000")
        self.check("round end stops player quotes", paused is not None and paused.status_code == 200 and not paused.json()["challenges"])
        await self.request(client, "paused_solve", "/challenges/c08/submit", "user000", data={"flag": "ferris{stress_08}"}, expected=(404,))
        await self.request(client, "next_round", "/admin/rounds/start", self.operator, data={"action": "start", "time_limit_minutes": 90}, expected=(303,))
        resumed = await self.request(client, "resumed_quotes", "/api/challenges/points", "user000")
        self.check("next round serves assigned challenges", resumed is not None and resumed.status_code == 200 and len(resumed.json()["challenges"]) == 15)
        with self.connect() as db:
            self.check("next-round pools exclude solved challenges", not db.execute("SELECT 1 FROM team_challenge_pool p JOIN solves s ON s.team_id=p.team_id AND s.challenge_id=p.challenge_id WHERE p.round_number=2").fetchall())
            self.check("SQLite integrity", db.execute("PRAGMA integrity_check").fetchone()[0] == "ok")
            self.check("foreign keys", not db.execute("PRAGMA foreign_key_check").fetchall())
            self.check("prices remain within configured bounds", not db.execute("SELECT 1 FROM challenge_pricing p JOIN challenges c ON c.id=p.challenge_id JOIN difficulty_scoring_rules r ON r.difficulty=c.difficulty WHERE p.anchor_units < r.minimum_points*1000000 OR p.anchor_units > r.maximum_points*1000000").fetchall())
            saved = db.execute("SELECT * FROM challenge_pricing ORDER BY challenge_id").fetchall()
            awards = db.execute("SELECT * FROM solves ORDER BY team_id,challenge_id").fetchall()
        self.stop()
        await self.start()
        await self.request(client, "restart_quote", "/api/challenges/points", "user000")
        with self.connect() as db:
            self.check("restart preserves pricing state and awards", saved == db.execute("SELECT * FROM challenge_pricing ORDER BY challenge_id").fetchall()
                and awards == db.execute("SELECT * FROM solves ORDER BY team_id,challenge_id").fetchall())
        with self.connect() as db:
            db.execute("UPDATE users SET is_banned=1 WHERE id='user098'")
        await self.request(client, "ban_revokes_access", "/api/challenges/points", "user098", expected=(303,))

    async def run(self):
        started = time.perf_counter()
        throughput = None
        try:
            await self.start()
            self.seed()
            await self.login_flows()
            limits = httpx.Limits(max_connections=self.args.concurrency, max_keepalive_connections=self.args.concurrency)
            async with httpx.AsyncClient(timeout=30, follow_redirects=False, limits=limits, trust_env=False) as client:
                await self.security(client)
                throughput = await self.reads(client)
                print("Concurrent solves, polls, hints and live admin edits", flush=True)
                await self.solving(client)
                await self.integrity(client)
        except Exception as error:
            self.failures.append({"harness_error": repr(error)})
        finally:
            self.stop()
        def percentiles(values):
            ordered = sorted(values)
            return {"requests": len(values), "p50_ms": round(ordered[int((len(ordered)-1)*.5)], 2),
                "p95_ms": round(ordered[int((len(ordered)-1)*.95)], 2), "p99_ms": round(ordered[int((len(ordered)-1)*.99)], 2),
                "max_ms": round(max(values), 2)}
        report = {"binary_sha256": self.binary_hash, "mode": "static" if self.args.static else "dynamic", "teams": 50, "participants": 100,
            "assigned_challenges": 45, "concurrency": self.args.concurrency, "target_read_rps": self.args.rps,
            "duration_seconds": round(time.perf_counter()-started, 2), "read_requests_per_second": round(throughput, 2) if throughput else None,
            "statuses": dict(self.codes), "latencies": {k: percentiles(v) for k,v in self.metrics.items()},
            "checks": self.checks, "unexpected_responses": self.failures[:30], "unexpected_response_count": len(self.failures)}
        report["passed"] = not self.failures and all(c["passed"] for c in self.checks)
        (self.output / "report.json").write_text(json.dumps(report, indent=2))
        print(json.dumps(report, indent=2), flush=True)
        return report["passed"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/ferrisctf")
    parser.add_argument("--output", default=str(Path(tempfile.gettempdir()) / ("ferri-stress-" + secrets.token_hex(4))))
    parser.add_argument("--reads", type=int, default=5000)
    parser.add_argument("--concurrency", type=int, default=100)
    parser.add_argument("--rps", type=float, default=0, help="Paced read requests/second; zero means unbounded burst")
    parser.add_argument("--static", action="store_true")
    args = parser.parse_args()
    if args.reads < 1 or args.concurrency < 1 or args.rps < 0:
        parser.error("reads and concurrency must be positive; rps must be nonnegative")
    raise SystemExit(0 if asyncio.run(Stress(args).run()) else 1)


if __name__ == "__main__":
    main()
