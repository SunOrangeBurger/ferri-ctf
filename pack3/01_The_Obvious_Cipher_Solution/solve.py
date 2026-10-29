from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '01_The_Obvious_Cipher_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

artifact=(PART/'complaint.txt').read_bytes()
assert artifact
assert FLAG.startswith('nexus{') and FLAG.endswith('}')
print(FLAG)
