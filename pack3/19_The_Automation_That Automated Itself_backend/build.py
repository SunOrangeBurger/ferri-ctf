from pathlib import Path
import json, base64
HERE=Path(__file__).resolve().parent
OUT=HERE/'rebuilt'
OUT.mkdir(exist_ok=True)
payload=json.loads((HERE/'source_artifacts.json').read_text())
for name, encoded in payload.items():
    (OUT/name).write_bytes(base64.b64decode(encoded))
print(f'Rebuilt {len(payload)} participant artifact(s) in {OUT}')
