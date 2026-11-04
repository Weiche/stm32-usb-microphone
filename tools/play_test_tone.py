"""Play a bounded, low-amplitude 1 kHz acoustic stimulus using the default Windows output."""
from array import array
import argparse
import math
from pathlib import Path
import sys
import wave
import winsound

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--seconds',type=int,default=20)
args=parser.parse_args()
if not 1<=args.seconds<=60: parser.error('Use 1..60 seconds')
path=Path('captures/stimulus-1000hz.wav')
path.parent.mkdir(exist_ok=True)
samples=array('h',(round(1600*math.sin(2*math.pi*1000*n/48000)) for n in range(48000*args.seconds)))
if sys.byteorder!='little': samples.byteswap()
with wave.open(str(path),'wb') as f:
    f.setparams((1,2,48000,0,'NONE','not compressed')); f.writeframes(samples.tobytes())
print('Playing 1 kHz test stimulus, digital peak -26.2 dBFS, duration',args.seconds,flush=True)
winsound.PlaySound(str(path.resolve()),winsound.SND_FILENAME)
