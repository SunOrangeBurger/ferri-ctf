from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '13_Incident_At_Byte_Bistro_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

chat=(PART/'chat.txt').read_text()
log=(PART/'access.log').read_text()
assert '90 seconds fast' in log and 'deploy started' in chat
assert FLAG=='nexus{the_clock_was_wrong}'
print(FLAG)
