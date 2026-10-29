from pathlib import Path
import json,csv,hashlib,struct,zlib,codecs,wave
H=Path(__file__).resolve().parent
P=H.parent/'13_Cipher_of_the_Canteen_Participants'
F=(H/'flag.txt').read_text().strip()
d=json.loads((P/'cipher.json').read_text()); key=b'cafeteria'; decoded=bytes(b^key[i%len(key)] for i,b in enumerate(bytes.fromhex(d['ciphertext_hex']))).decode(); assert decoded==F; print(decoded)
