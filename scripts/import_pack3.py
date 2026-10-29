#!/usr/bin/env python3
"""Import pack3 into the configured local SQLite DB; reruns verify and skip imports.

Run from the repository root: python3 scripts/import_pack3.py
Only Participants files are packaged. Unusual difficulty maps to hard.
"""
import hashlib
import hmac
import io
import json
import os
from pathlib import Path
import secrets
import shlex
import sqlite3
import uuid
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main(pack="pack3", expected_count=20):
    env = {}
    for line in (ROOT / '.env').read_text().splitlines():
        if line.strip() and not line.lstrip().startswith('#'):
            key, value = line.split('=', 1)
            parts = shlex.split(value, comments=True)
            env[key.strip()] = parts[0] if parts else ''
    env.update(os.environ)
    secret = env['SERVER_SECRET'].encode()
    if len(secret) < 32:
        raise ValueError('SERVER_SECRET must contain at least 32 bytes')
    url = env.get('DATABASE_URL', 'sqlite://ferrisctf.db?mode=rwc')
    if not url.startswith('sqlite://'):
        raise ValueError('Only local sqlite:// databases are supported')
    db_path = Path(url[len('sqlite://'):].split('?', 1)[0])
    if not db_path.is_absolute():
        db_path = ROOT / db_path
    if not db_path.is_file():
        raise ValueError('Configured database must already exist and be migrated')
    flag_key = hmac.digest(secret, b'ferrisctf/v1/flag', 'sha256')
    prepared = []
    for metadata in sorted((ROOT / pack).glob('*_backend/challenge.json')):
        data = json.loads(metadata.read_text())
        stem = metadata.parent.name.removesuffix('_backend')
        participant = ROOT / pack / (stem + '_Participants')
        flag = (ROOT / pack / (stem + '_Solution') / 'flag.txt').read_text().strip()
        if not flag:
            raise ValueError(f'Empty flag: {stem}')
        description = (participant / 'README.md').read_text().strip()
        difficulty = data['difficulty'].lower()
        if difficulty == 'unusual':
            difficulty = 'hard'
        points = {'easy': 50, 'medium': 100, 'hard': 200}[difficulty]
        files = sorted(p for p in participant.rglob('*') if p.is_file())
        if any(p.is_symlink() for p in participant.rglob('*')):
            raise ValueError(f'Symlink in {participant}')
        archive = io.BytesIO()
        with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as z:
            for path in files:
                z.write(path, path.relative_to(participant).as_posix())
        payload = archive.getvalue()
        if len(payload) > int(env.get('MAX_UPLOAD_BYTES', 31457280)):
            raise ValueError(f'ZIP exceeds upload limit: {stem}')
        with zipfile.ZipFile(io.BytesIO(payload)) as z:
            assert z.testzip() is None
            assert set(z.namelist()) == {p.relative_to(participant).as_posix() for p in files}
            assert all(z.read(p.relative_to(participant).as_posix()) == p.read_bytes() for p in files)
        prepared.append((stem, data, flag, description, difficulty, points, payload))
    if len(prepared) != expected_count:
        raise ValueError(f'Expected {expected_count} challenges, found {len(prepared)}')
    created_files = []
    inserted = skipped = 0
    db = sqlite3.connect(db_path, timeout=30)
    db.execute('PRAGMA foreign_keys = ON')
    has_backlog_round = any(row[1] == 'is_backlog_round' for row in db.execute('PRAGMA table_info(rounds)'))
    has_last_round = any(row[1] == 'is_last_round' for row in db.execute('PRAGMA table_info(rounds)'))
    try:
        db.execute('BEGIN IMMEDIATE')
        for stem, data, flag, description, difficulty, points, payload in prepared:
            cid = str(uuid.uuid5(uuid.NAMESPACE_URL, 'ferrisctf/' + pack + '/' + stem))
            existing = db.execute('SELECT flag_salt, flag_hmac, file_path FROM challenges WHERE id = ?', (cid,)).fetchone()
            if existing:
                digest = hmac.new(flag_key, (existing[0] + flag).encode(), hashlib.sha256).hexdigest()
                if not hmac.compare_digest(digest, existing[1]) or Path(existing[2]).read_bytes() != payload:
                    raise ValueError(f'Existing import differs: {stem}')
                skipped += 1
                continue
            if db.execute('SELECT 1 FROM challenges WHERE title = ?', (data['title'],)).fetchone():
                raise ValueError(f'Title already exists: {data["title"]}')
            # Pack4's final entry has only a template and a reference marker,
            # rather than a playable static flag challenge.
            active = not (pack == 'pack4' and stem == '30_CaughtRedHanded')
            salt = secrets.token_hex(16)
            digest = hmac.new(flag_key, (salt + flag).encode(), hashlib.sha256).hexdigest()
            uploads = ROOT / 'uploads'
            uploads.mkdir(exist_ok=True)
            path = uploads / (str(uuid.uuid4()) + '.zip')
            created_files.append(path)
            with path.open('xb') as out:
                out.write(payload)
            db.execute('''INSERT INTO challenges
                (id,title,description,category,difficulty,points,flag_hmac,flag_salt,file_path,file_name,is_active,is_selective)
                VALUES (?,?,?,?,?,?,?,?,?,?,?,0)''',
                (cid, data['title'], description, data['category'], difficulty, points, digest, salt,
                 str(path), stem.replace(' ', '_') + '_Participants.zip', int(active)))
            # Match the admin upload route: expose new standard challenges to locked teams in an active standard round.
            last_round_filter = 'AND r.is_last_round=0' if has_last_round else ''
            if has_backlog_round:
                last_round_filter += ' AND r.is_backlog_round=0'
            db.execute(f'''INSERT OR IGNORE INTO team_challenge_pool
                (team_id,round_number,challenge_id,repeat_penalty,opportunity_points)
                SELECT t.id,r.round_number,c.id,0,
                    CASE WHEN s.enabled=1 THEN rules.maximum_points ELSE c.points END
                FROM teams t CROSS JOIN rounds r CROSS JOIN challenges c
                CROSS JOIN scoring_settings s
                LEFT JOIN difficulty_scoring_rules rules ON rules.difficulty=c.difficulty
                WHERE t.is_locked=1 AND t.disbanded=0 AND r.ended_at IS NULL
                    AND r.is_selective_only=0 AND c.is_active=1
                    AND (r.auto_end_at IS NULL OR r.auto_end_at > strftime('%Y-%m-%d %H:%M:%f','now'))
                    {last_round_filter} AND c.id=?''', (cid,))
            inserted += 1
        db.commit()
    except BaseException:
        db.rollback()
        for path in created_files:
            path.unlink(missing_ok=True)
        raise
    finally:
        db.close()
    print(f'{pack}: imported {inserted}, verified and skipped {skipped}; participant-only ZIPs checked.')


if __name__ == '__main__':
    main()
