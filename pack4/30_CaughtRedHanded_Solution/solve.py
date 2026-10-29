from pathlib import Path
import json,csv,hashlib,struct,zlib,codecs,wave
H=Path(__file__).resolve().parent
P=H.parent/'30_CaughtRedHanded_Participants'
F=(H/'flag.txt').read_text().strip()
assert (P/'case_card_template.md').exists(); print('Reference marker only; score per-team private case record.')
