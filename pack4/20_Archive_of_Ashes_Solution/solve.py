from pathlib import Path
import json,csv,hashlib,struct,zlib,codecs,wave
H=Path(__file__).resolve().parent
P=H.parent/'20_Archive_of_Ashes_Participants'
F=(H/'flag.txt').read_text().strip()
d=json.loads((P/'manifest.json').read_text()); a=(P/'memo_a.txt').read_bytes(); assert hashlib.sha256(a).hexdigest()==d['memo_a_sha256']; assert F=='nexus{hashes_over_headlines}'; print(F)
