from pathlib import Path
import json,csv,hashlib,struct,zlib,codecs,wave
H=Path(__file__).resolve().parent
P=H.parent/'25_The_Overconfident_Bot_Participants'
F=(H/'flag.txt').read_text().strip()
decoded=codecs.decode((P/'incident.txt').read_text().strip(),'rot_13'); assert decoded==F; print(decoded)
