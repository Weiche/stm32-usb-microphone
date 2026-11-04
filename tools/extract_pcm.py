"""Extract the MCU-decoded finite PCM snapshot and measure amplitude/DC/spectrum."""
import argparse
from array import array
import json
import math
from pathlib import Path
import re
import sys
import wave


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('log',type=Path)
    parser.add_argument('--wav',type=Path,required=True)
    parser.add_argument('--json',type=Path,required=True)
    args=parser.parse_args()
    log=args.log.read_text(encoding='utf-8-sig')
    samples=[]
    for match in re.finditer(r'pcm_raw index=(\d+) data=\[([^\]]*)\]',log):
        if int(match[1])!=len(samples): raise ValueError('Missing or reordered PCM RTT block')
        chunk=[int(x.strip()) for x in match[2].split(',') if x.strip()]
        if len(chunk)!=16 or any(not -32768<=x<=32767 for x in chunk): raise ValueError('Invalid PCM block')
        samples.extend(chunk)
    if len(samples)!=1024 or 'pcm_snapshot end samples=1024' not in log: raise ValueError('Incomplete PCM snapshot')
    dc=sum(samples)/len(samples)
    rms=math.sqrt(sum((x-dc)**2 for x in samples)/len(samples))
    def amplitude(frequency):
        w=2*math.pi*frequency/16000
        real=sum((x-dc)*math.cos(w*i) for i,x in enumerate(samples))
        imag=sum((x-dc)*math.sin(w*i) for i,x in enumerate(samples))
        return 2*math.hypot(real,imag)/len(samples)
    bins={i*16000/len(samples):amplitude(i*16000/len(samples)) for i in range(3,513)}
    peak_frequency=max(bins,key=bins.get)
    report={'samples':len(samples),'rate_hz':16000,'duration_seconds':len(samples)/16000,
            'peak_lsb':max(abs(x) for x in samples),'rms_ac_lsb':rms,'dc_lsb':dc,
            'spectral_peak_hz':peak_frequency,'fft_bin_spacing_hz':16000/len(samples),
            'tone_1000_amplitude_lsb':amplitude(1000),'clipped_samples':sum(x in (-32768,32767) for x in samples),
            'note':'64 ms diagnostic snapshot; acoustic gain and absolute frequency are uncalibrated.'}
    args.wav.parent.mkdir(parents=True,exist_ok=True)
    packed=array('h',samples)
    if sys.byteorder!='little': packed.byteswap()
    with wave.open(str(args.wav),'wb') as f:
        f.setparams((1,2,16000,0,'NONE','not compressed')); f.writeframes(packed.tobytes())
    args.json.write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(report,indent=2))


if __name__=='__main__': main()
