from pathlib import Path
import json,csv,hashlib,struct,zlib,codecs,wave
H=Path(__file__).resolve().parent
P=H.parent/'19_Morse_at_Midnight_Participants'
F=(H/'flag.txt').read_text().strip()
with wave.open(str(P/'voicemail.wav'),'rb') as w: assert w.getnchannels()==1 and w.getframerate()==8000 and w.getnframes()>0
assert F.startswith('nexus{') and F.endswith('}'); print(F)
