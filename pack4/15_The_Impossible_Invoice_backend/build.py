from pathlib import Path
import json,base64
H=Path(__file__).resolve().parent; O=H/'rebuilt'; O.mkdir(exist_ok=True)
for n,b in json.loads((H/'source_artifacts.json').read_text()).items():
 p=O/n; p.parent.mkdir(parents=True,exist_ok=True); p.write_bytes(base64.b64decode(b))
print('Rebuilt participant artifacts under',O)
