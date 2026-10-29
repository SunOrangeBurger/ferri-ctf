from pathlib import Path
import json,csv,hashlib,struct,zlib,codecs,wave
H=Path(__file__).resolve().parent
P=H.parent/'21_CSV_Confessional_Participants'
F=(H/'flag.txt').read_text().strip()
r=list(csv.DictReader((P/'fragments.csv').open())); decoded=''.join(x['fragment'] for x in sorted(r,key=lambda x:int(x['offset']))); assert decoded==F; print(decoded)
