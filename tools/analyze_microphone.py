"""Summarize real microphone WAV without applying the software-tone amplitude assumptions."""
import argparse
from array import array
import json
import math
from pathlib import Path
import sys
import wave


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('wav',type=Path)
    parser.add_argument('--json',type=Path,required=True)
    args=parser.parse_args()
    with wave.open(str(args.wav),'rb') as f:
        if (f.getnchannels(),f.getsampwidth(),f.getframerate(),f.getcomptype())!=(1,2,16000,'NONE'):
            raise ValueError('Expected mono 16kHz signed 16-bit PCM WAV')
        samples=array('h',f.readframes(f.getnframes()))
    if sys.byteorder!='little':samples.byteswap()
    sine=[math.sin(2*math.pi*i/16) for i in range(16)]
    cosine=[math.cos(2*math.pi*i/16) for i in range(16)]
    windows=[]
    for start in range(0,len(samples)-15999,16000):
        data=samples[start:start+16000]; dc=sum(data)/16000
        rms=math.sqrt(sum((x-dc)**2 for x in data)/16000)
        re=sum(x*cosine[i%16] for i,x in enumerate(data))
        im=sum(x*sine[i%16] for i,x in enumerate(data))
        windows.append({'start_second':start//16000,'dc_lsb':dc,'rms_ac_lsb':rms,
                        'peak_lsb':max(abs(x) for x in data),'tone_1000_amplitude_lsb':2*math.hypot(re,im)/16000})
    zero_ms=sum(not any(samples[i:i+16]) for i in range(3200,len(samples)-15,16))
    report={'file':str(args.wav),'samples':len(samples),'duration_seconds':len(samples)/16000,
            'clipped_samples':sum(x in (-32768,32767) for x in samples),
            'zero_ms_after_first_200ms':zero_ms,'windows':windows,
            'limits':'Room audio is uncontrolled. These metrics do not establish frequency response, calibrated gain, or absence of every USB packet loss.'}
    args.json.parent.mkdir(parents=True,exist_ok=True)
    args.json.write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({**{k:v for k,v in report.items() if k!='windows'},'first_windows':windows[:3],'last_windows':windows[-2:]},indent=2))


if __name__=='__main__':main()
