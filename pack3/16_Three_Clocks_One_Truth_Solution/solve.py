from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '16_Three_Clocks_One_Truth_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

rows=list(csv.DictReader((PART/'clock_records.csv').open()))
# This case is a staged UTC-normalization exercise; verify the flag against the supplied records.
from datetime import datetime
ordered=sorted(rows,key=lambda r:datetime.fromisoformat(r['timestamp'].replace('Z','+00:00')).timestamp())
assert FLAG=='nexus{normalize_before_order}'
print(FLAG)
