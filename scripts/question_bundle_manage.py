#!/usr/bin/env python3
"""Local organizer helper copied into a generated, private question bundle."""
import argparse
import csv
import importlib
import json
import os
from pathlib import Path
import secrets
import sqlite3
import subprocess
import sys
from urllib.parse import urlsplit


ROOT = Path(__file__).resolve().parent


def load_question(question):
    if question not in [f"Q{n:02}" for n in range(1, 16)]:
        raise ValueError("Choose Q01 through Q15, or all where supported.")
    folder = ROOT / question
    record = json.loads((folder / "challenge.json").read_text())
    backend = folder / "source" / record["backend_directory"]
    if backend.parent != folder / "source" or not backend.is_dir():
        raise ValueError("Invalid backend directory in challenge.json.")
    if record["number"] != int(question[1:]):
        raise ValueError("Question number does not match its folder")
    if 11 <= record["number"] <= 14 and record["expected_flag"] != (backend / "flag.txt").read_text().strip():
        raise ValueError(f"{question}: static backend flag does not match challenge.json; preserve source evidence and flag")
    return folder, record, backend


def questions(selection):
    return [f"Q{n:02}" for n in range(1, 16)] if selection == "all" else [selection]


def check(selection, for_upload):
    report = []
    for question in questions(selection):
        folder, record, backend = load_question(question)
        pending = []
        points = record["points"]
        if type(points) is not int or points < 0:
            pending.append("choose non-negative integer points")
        if record["difficulty"] not in ("easy", "medium", "hard") or not record["difficulty_reviewed"]:
            pending.append("review difficulty and round-pool allocation")
        if record["mode"] not in ("hosted", "offline") or (record["mode"] == "offline" and question != "Q14"):
            raise ValueError(f"{question}: unsupported delivery mode")
        if record["mode"] == "hosted":
            url = record["challenge_url"]
            parsed = urlsplit(url)
            if (parsed.scheme not in ("http", "https") or not parsed.hostname
                    or parsed.username or parsed.password or "\\" in url
                    or any(c.isspace() or ord(c) < 32 for c in url)):
                pending.append("set a reachable HTTP(S) challenge_url")
        attachment = ROOT.parent / "uploads" / record["attachment"]
        if not attachment.is_file() or not record["expected_flag"].startswith("nexus{"):
            raise ValueError(f"{question}: missing attachment or flag")
        report.append({"question": question, "package_present": True, "mode": record["mode"],
                       "pending_upload_settings": pending, "wave": record["wave"]})
    print(json.dumps(report, indent=2))
    return int(for_upload and any(row["pending_upload_settings"] for row in report))


def backend_environment(folder, record):
    env = os.environ.copy()
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    if record["number"] <= 10:
        # A persistent secret is required for the same team to keep the same case.
        env["VAULT_CTF_SECRET"] = (folder / "backend-secret.txt").read_text().strip()
    else:
        # Keep the bundle's state local even if a shell has unrelated DB settings.
        env.pop("CTF_DB", None)
        env.pop("BLACKOUT_DB", None)
    return env


def read_roster(path):
    with path.open(newline="", encoding="utf-8-sig") as stream:
        reader = csv.DictReader(stream)
        if reader.fieldnames != ["name"]:
            raise ValueError("Roster CSV must have exactly one column: name")
        names = [row["name"].strip() for row in reader]
    if not names or any(not name or len(name) > 80 for name in names) or len(names) != len(set(names)):
        raise ValueError("Supply unique team names of 1–80 characters.")
    return names


def provision_one(question, names):
    folder, record, backend = load_question(question)
    if record["mode"] == "offline":
        print(f"{question}: offline; no team code or server needed")
        return
    environment = backend_environment(folder, record)
    for key in ("CTF_DB", "BLACKOUT_DB"):
        os.environ.pop(key, None)
    os.environ.update(environment)
    sys.path.insert(0, str(backend))
    number = record["number"]
    database = backend / "runtime" / ("challenge.sqlite3" if number <= 10 else
                                     "blackout.sqlite3" if number == 15 else "event.sqlite3")
    codes_file = folder / "access-codes.csv"
    existing = {}
    if codes_file.exists():
        if not database.exists():
            raise ValueError(f"{question}: code roster exists but its database is missing; restore both from backup")
        with codes_file.open(newline="") as stream:
            existing = {row["name"]: row["code"] for row in csv.DictReader(stream)}
        with sqlite3.connect(database) as db:
            recorded_names = {row[0] for row in db.execute("SELECT name FROM teams")}
        if not set(existing).issubset(recorded_names):
            raise ValueError(f"{question}: code roster and database disagree; restore a matching backup")
    if number <= 10:
        engine = importlib.import_module("engine")
        engine.init_db(False)
        def create(name, code):
            return engine.create_team(name, code, record["expected_flag"])
    elif number == 15:
        app = importlib.import_module("blackout")
        config = app.Config.environment()
        app.initialize(config)
        def create(name, code):
            return app.new_team(config, name, code, record["expected_flag"])
    else:
        app = importlib.import_module("app")
        host = importlib.import_module("host")
        store = host.Store(database)
        def create(name, code):
            return store.team(name, app.initial, code)
    created = 0
    for name in names:
        if name in existing:
            continue
        code = f"{question}-" + secrets.token_urlsafe(24)
        create(name, code)
        # Append after each success so a partially completed batch can resume.
        new_file = not codes_file.exists()
        fd = os.open(codes_file, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        with os.fdopen(fd, "a", newline="") as stream:
            writer = csv.writer(stream)
            if new_file:
                writer.writerow(["name", "code"])
            writer.writerow([name, code])
        existing[name] = code
        created += 1
    print(f"{question}: created {created}; retained {len(names) - created}. Private codes: {codes_file}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    checks = sub.add_parser("check", help="Inspect local settings; never contacts a server")
    checks.add_argument("--question", default="all")
    checks.add_argument("--for-upload", action="store_true", help="Fail if points, difficulty review or hosted URL are missing")
    provision = sub.add_parser("provision", help="Create teams only in the selected bundle's local databases")
    provision.add_argument("--question", default="all")
    choice = provision.add_mutually_exclusive_group(required=True)
    choice.add_argument("--roster", type=Path, help="CSV with one column, name")
    choice.add_argument("--name", help="Create one rehearsal team")
    serve = sub.add_parser("serve", help="Run one question in the foreground; loopback by default")
    serve.add_argument("--question", required=True)
    serve.add_argument("--host", default="127.0.0.1")
    serve.add_argument("--port", type=int)
    args = parser.parse_args()
    os.umask(0o077)
    if args.command == "check":
        return check(args.question, args.for_upload)
    if args.command == "provision":
        names = read_roster(args.roster) if args.roster else [args.name.strip()]
        if any(not name or len(name) > 80 for name in names):
            raise ValueError("Team names must contain 1–80 characters")
        if args.question == "all":
            # One interpreter per backend avoids module-name collisions; teams are batched.
            for question in questions("all"):
                command = [sys.executable, str(Path(__file__).resolve()), "provision", "--question", question]
                command += ["--roster", str(args.roster.resolve())] if args.roster else ["--name", args.name]
                result = subprocess.run(command, check=False)
                if result.returncode:
                    return result.returncode
        else:
            provision_one(args.question, names)
        return 0
    folder, record, backend = load_question(args.question)
    if record["mode"] == "offline":
        raise ValueError("Q14 uses its offline evidence ZIP; no service is necessary")
    port = args.port if args.port is not None else record["default_port"]
    if not 1 <= port <= 65535:
        raise ValueError("Port must be between 1 and 65535")
    command = [sys.executable, str(backend / record["entrypoint"]), "serve", "--host", args.host, "--port", str(port)]
    print(f"Serving {args.question} on {args.host}:{port}; stop with Ctrl-C", flush=True)
    os.chdir(backend)
    os.execvpe(sys.executable, command, backend_environment(folder, record))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError, sqlite3.Error) as error:
        raise SystemExit(f"Setup error: {error}")
