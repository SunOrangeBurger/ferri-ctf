from pathlib import Path
import json,csv,hashlib,struct,zlib,codecs,wave
H=Path(__file__).resolve().parent
P=H.parent/'11_Logjam_at_09_17_Participants'
F=(H/'flag.txt').read_text().strip()
assert F.startswith('nexus{') and F.endswith('}') and any(P.iterdir()); print(F)
