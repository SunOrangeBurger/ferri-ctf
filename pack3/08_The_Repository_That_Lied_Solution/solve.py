from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '08_The_Repository_That_Lied_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

artifact=(PART/'reporter.py').read_bytes()
assert artifact
assert FLAG.startswith('nexus{') and FLAG.endswith('}')
print(FLAG)
