from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '19_The_Automation_That Automated Itself_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

text=(PART/'bot_report.txt').read_text().split('Encoded incident note:\n',1)[1].strip()
decoded=codecs.decode(text,'rot_13')
assert decoded==FLAG,(decoded,FLAG)
print(decoded)
