from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '20_The_Mystery_Of_Missing_Evidence_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

artifact=(PART/'casefile.txt').read_bytes()
assert artifact
assert FLAG.startswith('nexus{') and FLAG.endswith('}')
print(FLAG)
