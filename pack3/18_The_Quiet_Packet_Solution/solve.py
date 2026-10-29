from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '18_The_Quiet_Packet_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

data=json.loads((PART/'packets.json').read_text())
decoded=''.join(x['payload'] for x in sorted(data['fragments'],key=lambda x:x['seq']))
assert hashlib.sha256(decoded.encode()).hexdigest()==data['sha256']
assert decoded==FLAG
print(decoded)
