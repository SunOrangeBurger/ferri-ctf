import argparse,hashlib,json
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--team',required=True); p.add_argument('--seed',required=True); p.add_argument('--out',default='private_case.json')
a=p.parse_args()
d=hashlib.sha256((a.seed+'|'+a.team).encode()).hexdigest()
record={'team':a.team,'case_id':d[:8].upper(),'nonce':d[8:20],
'expected_response':'nexus{case_'+hashlib.sha256((d+'|evidence').encode()).hexdigest()[:16]+'}',
'verification':'Ask team to identify two exact evidence positions in its issued variant.'}
Path(a.out).write_text(json.dumps(record,indent=2))
print('Private case generated. Do not distribute the JSON.')
