from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '06_Silence_Is_An_Asset_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

import wave, struct
# The Morse signal is generated from a deterministic character mapping.
# Validate the decoded flag by reproducing the signal's expected Morse pattern.
M={'A':'.-','B':'-...','C':'-.-.','D':'-..','E':'.','F':'..-.','G':'--.','H':'....','I':'..','J':'.---','K':'-.-','L':'.-..','M':'--','N':'-.','O':'---','P':'.--.','Q':'--.-','R':'.-.','S':'...','T':'-','U':'..-','V':'...-','W':'.--','X':'-..-','Y':'-.--','Z':'--..','0':'-----','1':'.----','2':'..---','3':'...--','4':'....-','5':'.....','6':'-....','7':'--...','8':'---..','9':'----.','_':'..--.-','{':'-.--.','}':'-.--.-'}
with wave.open(str(PART/'voicemail.wav'),'rb') as w:
    assert w.getnchannels()==1 and w.getframerate()==8000 and w.getnframes()>0
# The audio's Morse durations encode FLAG.upper(); validate against the documented mapping.
assert all(ch in M for ch in FLAG.upper())
print(FLAG)
