from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '11_CSV_Of_Doom_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

rows=list(csv.DictReader((PART/'fragments.csv').open()))
decoded=''.join(r['fragment'] for r in sorted(rows,key=lambda r:int(r['position'])))
assert decoded==FLAG,(decoded,FLAG)
print(decoded)
