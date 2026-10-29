from pathlib import Path
import json,csv,hashlib,struct,zlib,codecs,wave
H=Path(__file__).resolve().parent
P=H.parent/'14_Packet_Pantry_Participants'
F=(H/'flag.txt').read_text().strip()
d=json.loads((P/'packets.json').read_text()); decoded=''.join(x['payload'] for x in sorted(d['fragments'],key=lambda x:x['seq'])); assert decoded==F and hashlib.sha256(decoded.encode()).hexdigest()==d['sha256']; print(decoded)
