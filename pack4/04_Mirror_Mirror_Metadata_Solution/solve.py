from pathlib import Path
import json,csv,hashlib,struct,zlib,codecs,wave
H=Path(__file__).resolve().parent
P=H.parent/'04_Mirror_Mirror_Metadata_Participants'
F=(H/'flag.txt').read_text().strip()

def png(p):
 d=p.read_bytes(); pos=8; dat=b''; w=h=ct=None
 while pos<len(d):
  ln=struct.unpack('>I',d[pos:pos+4])[0]; tag=d[pos+4:pos+8]; b=d[pos+8:pos+8+ln]; pos+=12+ln
  if tag==b'IHDR': w,h,dep,ct,co,fi,it=struct.unpack('>IIBBBBB',b)
  elif tag==b'IDAT': dat+=b
 ch=4 if ct==6 else 3; raw=zlib.decompress(dat); px=[]; off=0
 for y in range(h):
  off+=1; row=raw[off:off+w*ch]; off+=w*ch; px.extend([tuple(row[x:x+ch]) for x in range(0,len(row),ch)])
 return w,h,px
def dec(bits):
 out=bytearray()
 for i in range(0,len(bits)-7,8):
  b=int(bits[i:i+8],2)
  if b==0: break
  out.append(b)
 return out.decode()
w,h,px=png(P/'evidence.png')
ordered=[]
for y in range(h): ordered.extend(px[y*w:(y+1)*w][::-1])
bits=''.join(str(p[1]&1) for p in ordered)
decoded=dec(bits)
assert decoded==F,(decoded,F)
print(decoded)
