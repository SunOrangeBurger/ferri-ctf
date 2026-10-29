from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '14_QR_Without_The_R_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

groups=(PART/'grid.txt').read_text().split()
decoded=bytes(int(x,2) for x in groups).decode()
assert decoded==FLAG,(decoded,FLAG)
print(decoded)
