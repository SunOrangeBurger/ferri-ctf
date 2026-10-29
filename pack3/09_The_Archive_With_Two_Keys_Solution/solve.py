from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '09_The_Archive_With_Two_Keys_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

d=json.loads((PART/'cipher.json').read_text())
key=''.join(x['fragment'] for x in sorted(d['key_fragments'],key=lambda x:x['time'])).encode()
cipher=bytes.fromhex(d['ciphertext_hex'])
decoded=bytes(b^key[i%len(key)] for i,b in enumerate(cipher)).decode()
assert decoded==FLAG,(decoded,FLAG)
print(decoded)
