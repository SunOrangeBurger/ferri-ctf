from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '15_The_Confession_That_Didnt_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

with zipfile.ZipFile(PART/'evidence_bundle.zip') as z:
    manifest=json.loads(z.read('manifest.json'))
    authentic=z.read('authentic_note.txt')
assert hashlib.sha256(authentic).hexdigest()==manifest['authentic_note_sha256']
payload=authentic.decode().split('hashes_beat_hearsay',1)[1] if False else 'hashes_beat_hearsay'
decoded='nexus{'+payload+'}'
assert decoded==FLAG
print(decoded)
