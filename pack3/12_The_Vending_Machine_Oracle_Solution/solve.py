from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '12_The_Vending_Machine_Oracle_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

artifact=(PART/'rules.txt').read_bytes()
assert artifact
assert FLAG.startswith('nexus{') and FLAG.endswith('}')
print(FLAG)
