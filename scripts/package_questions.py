#!/usr/bin/env python3
"""Prepare FINAL_QUESTIONS.zip locally. Never uploads, installs, or starts services."""
import argparse
import ast
import csv
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import secrets
import tempfile
import stat
import zipfile


EXCLUDED = {"__MACOSX", ".venv", "venv", "__pycache__", ".DS_Store", "runtime", ".git"}
CATEGORIES = {8: "Forensics", 11: "Web", 12: "Forensics", 13: "Crypto", 14: "Forensics", 15: "Rev"}
PORTS = {11: 8081, 12: 8084, 13: 8083, 14: 8082, 15: 8080}
PARTICIPANT_FILES = {
    11: {"README.md", "support_ticket.txt", "saved_cart_format.txt", "procurement_ledger.csv", "dispatch_history.jsonl"},
    12: {"README.md", "patch_log.csv", "engineering_note.txt", "night_shift.wav", "incident_note.txt"},
    13: {"README.md", "envelope_spec.txt", "batch_ledger.csv", "shift_note.txt"},
    14: {"README.md", "incident_note.txt", "shattered_backup.zip"},
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    path.write_bytes(data if isinstance(data, bytes) else data.encode())
    path.chmod(0o600)


def make_zip(entries):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for name, data in sorted(entries.items()):
            info = zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (stat.S_IFREG | 0o600) << 16
            archive.writestr(info, data)
    return buffer.getvalue()


def source_files(source):
    files = {}
    with zipfile.ZipFile(source) as archive:
        if sum(i.file_size for i in archive.infolist()) > 150_000_000:
            raise ValueError("Source archive exceeds the 150 MB expanded-size limit")
        for info in archive.infolist():
            path = PurePosixPath(info.filename)
            if path.is_absolute() or ".." in path.parts or "\\" in info.filename:
                raise ValueError("Unsafe source archive path")
            if info.is_dir() or any(p in EXCLUDED or p.startswith("._") for p in path.parts):
                continue
            if stat.S_ISLNK(info.external_attr >> 16):
                raise ValueError("Source archive contains a symlink outside excluded directories")
            if path.suffix in (".pyc", ".sqlite3", ".db") or path.name.endswith(("-wal", "-shm")):
                continue
            if info.filename in files:
                raise ValueError("Duplicate source archive member")
            files[info.filename] = archive.read(info)
    return files


def config_literals(data):
    values = {}
    # Parse constants without executing supplied Python.
    for node in ast.parse(data).body:
        if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            try:
                values[node.targets[0].id] = ast.literal_eval(node.value)
            except (ValueError, TypeError):
                pass
    return values


def participant_readme(number, original):
    text = original.decode()
    if number == 14:
        start = text.index("## Start here")
        end = text.index("`shattered_backup.zip`", start)
        text = text[:start] + ("## Start here\n\nDownload and extract this participant ZIP. "
                              "Use `shattered_backup.zip` as the evidence pack; no website or team code is required.\n\n") + text[end:]
        start = text.index("The hosted workbench gives")
        end = text.index("Restoring sectors", start)
        text = text[:start] + text[end:]
        text = text.replace("; the website also displays it after restoring that exact audit", "")
    else:
        # Remove development-only addresses from participant-facing instructions.
        text = re.sub(r"The local host default is `http://127\.0\.0\.1:\d+`; use the address supplied by your organizer when playing remotely\.", "Use the event address supplied by your organizer.", text)
        text = re.sub(r"Local default: `http://127\.0\.0\.1:\d+`\. Use the supplied event URL for remote play\.", "Use the supplied event URL.", text)
        text = re.sub(r"The local default is `http://127\.0\.0\.1:\d+`; remote players use the supplied event URL\.", "Use the supplied event URL.", text)
        text = text.replace("team-specific `nexus{...}` flag", "question's `nexus{...}` flag")
        title, rest = text.split("\n", 1)
        text = title + ("\n\n## Event access\n\nOpen this question in FerrisCTF and select **Open challenge website**, "
                        "or use the event URL announced by the organizer. `CHALLENGE_URL` in this brief means that address. "
                        "Sign in there with your question access code, issued privately to your team. "
                        "Your FerrisCTF login and question access code are separate. "
                        "Submit the revealed flag back in FerrisCTF.\n") + rest
    return text.encode()


def hosting_guide(record):
    q = record["id"]
    if record["mode"] == "offline":
        return f"""# {q}: offline delivery

Upload `uploads/{record['attachment']}` using the fields in `challenge.json` and
`description.md`. Choose points and review difficulty first. Keep Challenge
website empty. The original evidence is unchanged; its flag must remain unchanged.
No backend, process, dependency install, or team code is needed for this question.
The original optional workbench and solution are preserved privately in `source/`.
Do not upload this directory or `challenge.json` to participants.
"""
    deps = ("This question uses only Python's standard library.\n" if record["number"] <= 10 else
            "Use the shared virtual environment described in START_HERE.md (FastAPI and Uvicorn).\n")
    return f"""# {q}: {record['title']}

{deps}
Run from the extracted bundle's root with Python 3.11 or later:

```sh
python3 organizer/manage.py provision --question {q} --name 'Rehearsal Team'
python3 organizer/manage.py serve --question {q}
```

The second command stays in the foreground. On this computer open
`http://127.0.0.1:{record['default_port']}/` and use the code in
`organizer/{q}/access-codes.csv`. Stop with Ctrl-C. Repeating provisioning retains
already recorded teams and their budgets. Do not use rehearsal codes for the event.
For actual teams use a one-column CSV (`name`) with `provision --roster teams.csv`.

The helper provisions a consistent flag from `challenge.json` for FerrisCTF's
single-flag-per-question verifier. Generated team puzzles and budgets remain separate.
For Q11–Q14 it preserves the source's static flag and evidence. No packaged event
database, demo team, or old session is reused. Do not run original reset/rebuild commands
on a live event. Keep the private code CSV, runtime directory, and (where present)
`backend-secret.txt` together when backing up or moving the service.

For participant access arrange a reachable event URL, then set `challenge_url` in
`challenge.json` and in FerrisCTF. Loopback URLs are only for local rehearsal.
The helper binds to loopback by default; `serve --host ADDRESS --port PORT` changes
that only when explicitly supplied. Public hosting, TLS, proxy admission, service
supervision, and rented-server deployment are separate operator tasks. Preserve
the original backend's root paths; these apps are not configured for a `/qNN/` URL prefix.
Use a distinct hostname from FerrisCTF, preferably a separate hostname per question.

The original backend documentation is under `source/{record['backend_directory']}`.
For Q11–Q13 use `CTF_PUBLIC_ORIGIN`/`CTF_SECURE_COOKIES` for HTTPS; Q15 uses
`BLACKOUT_PUBLIC_ORIGIN`/`BLACKOUT_SECURE_COOKIES`. Direct-peer IP limits need a
separate check behind a shared proxy. This pack adds no capacity claim or wave gate.

After the URL works from a participant device, choose points, review difficulty
and pool allocation, upload `{record['attachment']}`, and copy `description.md`
and the private expected flag into the matching FerrisCTF fields. The JSON file is
a field reference; FerrisCTF does not currently import it automatically.
"""


START = """# Start here: 15 questions in Q01–Q15 order

This complete bundle is ORGANIZER ONLY. Do not upload or publish the outer ZIP.
The only participant uploads are the 15 individual ZIPs in `uploads/`.
`CTF-Participant-Uploads.zip` is a convenience collection to unpack first; uploading
that collection as one FerrisCTF challenge does not create 15 challenges.

## Files

- `INDEX.csv`: ordered inventory; blank points and waves are intentional.
- `UPLOAD_ORDER.md`: clickable question-by-question upload checklist.
- `uploads/Q01-Participants.zip` through `Q15-Participants.zip`: participant files.
- `organizer/QNN/challenge.json`: matching form fields, exact private flag, and editable settings.
- `organizer/QNN/description.md`: participant-facing description to paste into FerrisCTF.
- `organizer/QNN/HOSTING.md`: question-specific setup.
- `organizer/QNN/source/`: cleaned original backend and solution, organizer only.
- `organizer/manage.py`: local settings check, team provisioning, and foreground rehearsal.
- `SOURCE_INTEGRITY.json` and `SHA256SUMS`: provenance and checksums.

## Upload in order

1. Obtain the existing operator login and use an agreed rehearsal context.
2. Set each question's `points` (left null as requested), choose its difficulty,
   and set `difficulty_reviewed` true in its JSON. Difficulty is immutable after
   creation in this platform. The proposed `hard` field preserves source intent
   for hard/extreme questions and is provisional for the remaining questions.
3. For the 14 hosted questions, run the backend and obtain a participant-reachable
   URL first. Update `challenge_url`; Q14 is offline and needs no URL.
4. Run `python3 organizer/manage.py check --for-upload`. This validates local
   settings; it does not contact the website or certify network reachability.
5. In FerrisCTF Manage Challenges, create Q01, Q02, ... Q15 using the JSON fields,
   `description.md`, and the corresponding participant ZIP. Copy `expected_flag`
   into Flag; do not upload the JSON, backend, solution, or outer organizer ZIP.
6. Test the participant URL, private code, attachment, and solve-to-score path
   before opening the event. Static points are per question; if dynamic scoring
   is selected, its configured rules determine awarded points instead.

New challenges are active immediately in the current upload handler. Prepare
them before the event in an agreed context; deactivate as needed. Uploading a ZIP
does not start a website. This bundle does not write to FerrisCTF's database or
change the rented server, service, rounds, accounts, or settings.

## Rehearse locally

You need Python 3.11 or later. Q01–Q10 use the standard library; Q14 is offline.
For Q11–Q13 and Q15, one shared environment suffices:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r organizer/requirements.txt
.venv/bin/python organizer/manage.py provision --question Q11 --name 'Rehearsal Team'
.venv/bin/python organizer/manage.py serve --question Q11
```

Read `organizer/Q11/access-codes.csv`, open http://127.0.0.1:8081/ on this computer,
and enter that code. Stop with Ctrl-C. For Q01 the same commands work with plain
`python3` and port 9101. For Q14 extract its participant ZIP and work offline.
The shared rehearsal environment pins FastAPI 0.134.0 and Uvicorn 0.41.0 from
the Q11/Q13 requirements. Original per-question dependency files are retained.
The helper never binds publicly unless you explicitly supply a different host.
Read each `HOSTING.md` before public hosting. No dependency install or server
connection is performed by merely generating or unpacking this bundle.

For up to 75 actual teams copy `organizer/teams.example.csv` to `teams.csv` and
replace the sample names with the exact roster, one name per row. Then run:

```sh
.venv/bin/python organizer/manage.py provision --question all --roster teams.csv
```

This batches each backend locally, skips offline Q14, saves codes privately per
question, and retains teams already recorded by the helper. It starts no servers.
Distribute only each team's own codes. Use a separate extracted copy for event
state and for rehearsal; don't reset exhausted budgets in the live event.

## Difficulty, pools, ordering, and waves

Q01–Q15 preserves the supplied numbering; it is not a calibrated easiest-to-hardest
ranking. FerrisCTF orders player lists by difficulty and then title; upload order
alone does not change that behavior. Prefixes preserve numeric order within a tier.

The existing pool uses 60% easy / 30% medium / 10% hard, with no fallback for an
empty tier. A 15-slot pool reserves 9 easy, 5 medium, and 1 hard slot; it receives
only one question if all registered questions are
labelled hard. Registering these 15 does not guarantee all 15 are assigned to
every team. The operator must settle allocation before the event; do not relabel
all questions easy or change live pool settings as an incidental upload step.
Adding challenges does not update a pool already assigned for the running round.

`wave` is deliberately null in every JSON. It is a planning field, not an enforced
feature or an import command. Proper waves and direct-URL blocking still require
the separate platform/backend changes discussed. Hiding a FerrisCTF launch link
does not stop a saved backend URL from working. Distributing all participant ZIPs
early also releases their offline evidence early; release only the intended files.

## Load and verification limits

Use shared question services with separate team state. This pack creates no
per-player process, polling loop, container, proxy, or background worker. Q14
needs no backend. Each of the other 14 still consumes resources when hosted;
packaging and local checks do not establish 150-player capacity on the rented VPS.
`VERIFICATION.json`, when supplied, records exactly which local checks were run.

Answers, solutions, codes, private keys, and event state stay organizer-only.
Keep Q11–Q14 static flags and evidence unchanged. Q01–Q10 and Q15 receive a new
private fixed flag for the question; provisioning supplies it to each independent
team, matching FerrisCTF's verifier. Original answer notes are reference material,
not replacements for the `expected_flag` in the prepared record.
"""


def build(source, destination):
    if destination.exists():
        raise ValueError("Output already exists; choose a new directory to avoid overwriting flags or event state")
    files = source_files(source)
    destination.mkdir(parents=True, mode=0o700)
    provenance = []
    records = []
    private_flags = []
    for number in range(1, 16):
        q = f"Q{number:02}"
        prefix = f"FINAL_QUESTIONS/Question {number}/"
        selected = {n[len(prefix):]: b for n, b in files.items() if n.startswith(prefix)}
        roots = {PurePosixPath(n).parts[0] for n in selected}
        parts = {}
        for kind in ("Participants", "backend", "Solution"):
            matches = [name for name in roots if name.endswith("_" + kind)]
            if len(matches) != 1:
                raise ValueError(f"{q}: expected one {kind} directory")
            parts[kind] = matches[0]
        original_readme = selected[parts["Participants"] + "/README.md"]
        participant_files = {n.split("/", 1)[1]: b for n, b in selected.items() if n.startswith(parts["Participants"] + "/")}
        if set(participant_files) != PARTICIPANT_FILES.get(number, {"README.md"}):
            raise ValueError(f"{q}: participant file inventory changed; review before distribution")
        participant_files["README.md"] = participant_readme(number, original_readme)
        if number <= 10:
            literals = config_literals(selected[parts["backend"] + "/challenge.py"])
            title = literals["TITLE"]
            port = literals["PORT"]
        else:
            title = original_readme.decode().splitlines()[0].lstrip("# ")
            port = PORTS[number]
        flag = (selected[parts["backend"] + "/flag.txt"].decode().strip() if 11 <= number <= 14
                else f"nexus{{{q.lower()}_{secrets.token_hex(20)}}}")
        if not re.fullmatch(r"nexus\{[^\s{}]+\}", flag):
            raise ValueError(f"{q}: malformed organizer flag")
        private_flags.append(flag.encode())
        source_difficulty = next((line.strip() for line in original_readme.decode().splitlines()
                                  if "difficulty" in line.lower()), "Uncalibrated; see organizer notes")
        record = {"id": q, "number": number, "title": f"{q} — {title}",
                  "category": CATEGORIES.get(number, "Misc"), "source_difficulty": source_difficulty,
                  "difficulty": "hard", "difficulty_reviewed": False, "points": None,
                  "wave": None, "mode": "offline" if number == 14 else "hosted",
                  "challenge_url": "", "expected_flag": flag, "attachment": f"{q}-Participants.zip",
                  "default_port": port, "backend_directory": parts["backend"],
                  "entrypoint": "server.py" if number <= 10 else "blackout.py" if number == 15 else "app.py"}
        records.append(record)
        folder = destination / "organizer" / q
        write(folder / "challenge.json", json.dumps(record, indent=2, ensure_ascii=False) + "\n")
        write(folder / "description.md", participant_files["README.md"])
        write(folder / "HOSTING.md", hosting_guide(record))
        if number <= 10:
            write(folder / "backend-secret.txt", secrets.token_hex(32) + "\n")
        for name, data in selected.items():
            if name.startswith((parts["backend"] + "/", parts["Solution"] + "/")):
                write(folder / "source" / name, data)
            provenance.append({"question": q, "source": prefix + name, "source_sha256": sha(data),
                               "participant": name.startswith(parts["Participants"] + "/"),
                               "readme_adapted": name == parts["Participants"] + "/README.md"})
        upload = make_zip(participant_files)
        if len(upload) >= 30 * 1024 * 1024:
            raise ValueError(f"{q}: participant ZIP exceeds upload limit")
        write(destination / "uploads" / record["attachment"], upload)
    # Check player-facing bytes against every exact expected flag. Encrypted or
    # fragmented puzzle evidence is retained intentionally, with its original hash.
    for path in (destination / "uploads").glob("*.zip"):
        with zipfile.ZipFile(path) as archive:
            for member in archive.infolist():
                data = archive.read(member)
                if any(flag in data for flag in private_flags):
                    raise ValueError("An exact organizer flag appears in participant plaintext")
    write(destination / "START_HERE.md", START)
    write(destination / "organizer" / "manage.py", Path(__file__).with_name("question_bundle_manage.py").read_bytes())
    requirements = {files[next(n for n in files if n.startswith(f"FINAL_QUESTIONS/Question {q}/")
                               and n.count("/") == 3 and n.endswith("_backend/requirements.txt"))] for q in range(11, 16)}
    known_requirements = {b"fastapi==0.134.0\nuvicorn==0.41.0\n", b"fastapi==0.128.8\nuvicorn==0.39.0\n"}
    if not requirements.issubset(known_requirements):
        raise ValueError("Backend requirements changed; review before sharing one environment")
    write(destination / "organizer" / "requirements.txt", "fastapi==0.134.0\nuvicorn==0.41.0\n")
    write(destination / "organizer" / "teams.example.csv", "name\nExample Team Alpha\nExample Team Beta\n")
    buffer = io.StringIO(newline="")
    fields = ["id", "title", "category", "source_difficulty", "points", "wave", "mode", "attachment"]
    writer = csv.DictWriter(buffer, fieldnames=fields, extrasaction="ignore")
    writer.writeheader()
    writer.writerows(records)
    write(destination / "INDEX.csv", buffer.getvalue())
    upload_order = ["# Upload checklist: Q01–Q15\n", "Use one row per FerrisCTF challenge. Points are unset; hosted URLs and difficulty review are required before upload.\n",
                    "| Question | Participant ZIP | Private fields | Description | Setup |",
                    "| --- | --- | --- | --- | --- |"]
    for record in records:
        q = record["id"]
        upload_order.append(f"| {record['title']} | [ZIP](uploads/{record['attachment']}) | [JSON](organizer/{q}/challenge.json) | [Brief](organizer/{q}/description.md) | [Guide](organizer/{q}/HOSTING.md) |")
    write(destination / "UPLOAD_ORDER.md", "\n".join(upload_order) + "\n")
    write(destination / "SOURCE_INTEGRITY.json", json.dumps({"source_archive_sha256": sha(source.read_bytes()),
          "excluded": sorted(EXCLUDED) + ["*.pyc", "*.sqlite3", "*.db", "*-wal", "*-shm"],
          "files": provenance}, indent=2) + "\n")
    seal(destination)
    return len(records)


def seal(destination):
    entries = {p.relative_to(destination).as_posix(): p.read_bytes() for p in destination.rglob("*")
               if p.is_file() and p.name != "SHA256SUMS"}
    write(destination / "SHA256SUMS", "".join(f"{sha(data)}  {name}\n" for name, data in sorted(entries.items())))
    public = {name: data for name, data in entries.items() if name.startswith("uploads/")}
    write(destination.parent / "CTF-Participant-Uploads.zip", make_zip(public))
    private = {destination.name + "/" + name: data for name, data in entries.items()}
    private[destination.name + "/SHA256SUMS"] = (destination / "SHA256SUMS").read_bytes()
    write(destination.parent / "CTF-Organizer-Private.zip", make_zip(private))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    # Output archives are beside the folder; preserve existing deliveries too.
    if args.output.exists() or any((args.output.parent / name).exists() for name in ("CTF-Participant-Uploads.zip", "CTF-Organizer-Private.zip")):
        parser.error("Delivery archives already exist here; choose a new output parent")
    try:
        args.output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        # A failed build leaves no half-prepared delivery or partial flag set.
        with tempfile.TemporaryDirectory(prefix=".question-pack-", dir=args.output.parent) as temporary:
            staged = Path(temporary) / args.output.name
            count = build(args.source, staged)
            staged.rename(args.output)
            for name in ("CTF-Participant-Uploads.zip", "CTF-Organizer-Private.zip"):
                (staged.parent / name).rename(args.output.parent / name)
    except (ValueError, KeyError, OSError, zipfile.BadZipFile) as error:
        parser.exit(1, f"Packaging error: {error}\n")
    print(f"Prepared {count} questions in {args.output}; points and waves are unset. No server contacted.")


if __name__ == "__main__":
    main()
