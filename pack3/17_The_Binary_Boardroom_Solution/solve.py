from pathlib import Path
import json, base64, csv, hashlib, zipfile, re, codecs, struct, zlib
HERE=Path(__file__).resolve().parent
PART=HERE.parent / '17_The_Binary_Boardroom_Participants'
FLAG=(HERE/'flag.txt').read_text().strip()

# Decode the blue-channel least-significant bits from the PNG.
# Minimal PNG reader supports the generated RGB, non-interlaced files.
import zlib
def read_png(path):
    data=path.read_bytes(); pos=8; idat=b''; width=height=None
    while pos<len(data):
        n=struct.unpack('>I',data[pos:pos+4])[0]; tag=data[pos+4:pos+8]; block=data[pos+8:pos+8+n]; pos+=12+n
        if tag==b'IHDR': width,height,depth,ctype,comp,filt,inter=struct.unpack('>IIBBBBB',block)
        elif tag==b'IDAT': idat+=block
    raw=zlib.decompress(idat); bpp=3; stride=width*bpp; rows=[]; i=0; prev=bytearray(stride)
    for y in range(height):
        ft=raw[i]; i+=1; scan=bytearray(raw[i:i+stride]); i+=stride
        for x in range(stride):
            a=scan[x-3] if x>=3 else 0; b=prev[x]; c=prev[x-3] if x>=3 else 0
            if ft==1: scan[x]=(scan[x]+a)&255
            elif ft==2: scan[x]=(scan[x]+b)&255
            elif ft==3: scan[x]=(scan[x]+((a+b)//2))&255
            elif ft==4:
                p=a+b-c; pa=abs(p-a); pb=abs(p-b); pc=abs(p-c); pr=a if pa<=pb and pa<=pc else (b if pb<=pc else c); scan[x]=(scan[x]+pr)&255
        rows.append(scan); prev=scan
    return width,height,rows
w,h,rows=read_png(PART/'boardroom.png')
bits=''.join(str(row[x*3+2]&1) for row in rows for x in range(w))
out=bytes(int(bits[i:i+8],2) for i in range(0,len(bits)-7,8))
decoded=out.split(bytes([0]),1)[0].decode()
assert decoded==FLAG, (decoded, FLAG)
print(decoded)
