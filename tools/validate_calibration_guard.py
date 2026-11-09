"""Offline same-input startup/weak-speech comparison; never plays audio or flashes."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import wave

import numpy as np

from denoise_offline import RATE, noise_profile, rms, write_wav

ROOT = Path(__file__).resolve().parents[1]
HOST = 'x86_64-pc-windows-msvc'


def read_speech(path):
    with wave.open(str(path), 'rb') as source:
        if source.getsampwidth() != 2 or source.getnchannels() != 1:
            raise ValueError('Speech must be mono signed 16-bit PCM')
        rate = source.getframerate()
        values = np.frombuffer(source.readframes(source.getnframes()), dtype='<i2') / 32768.0
    return np.interp(np.arange(round(len(values) * RATE / rate)) * rate / RATE,
                     np.arange(len(values)), values)


def run(executable, directory, name, samples, profile=None, adaptive=False):
    source = directory / 'input.s16le'
    if not source.exists():
        source.write_bytes(np.clip(np.rint(samples * 32768), -32768, 32767).astype('<i2').tobytes())
    output = directory / (name + '.s16le')
    trace = directory / (name + '.csv')
    estimate = directory / (name + '.noise.f32le')
    mode = 'startup'
    if profile is not None:
        noise = directory / 'input.noise.f32le'
        noise.write_bytes(np.asarray(profile, dtype='<f4').tobytes())
        mode = str(noise)
    command = [str(executable), str(source), str(output), mode, '--trace', str(trace),
               '--profile-out', str(estimate)]
    if adaptive:
        command.append('--adaptive')
    subprocess.run(command, check=True, capture_output=True)
    frames = np.atleast_1d(np.genfromtxt(trace, names=True, delimiter=','))
    frames = frames[frames['input_samples'] <= len(samples)]
    actual = np.frombuffer(output.read_bytes(), dtype='<i2').astype(float) / 32768
    assert len(actual) == len(samples) and np.all(np.isfinite(actual))
    write_wav(directory / (name + '.wav'), actual)
    ready = frames[frames['ready'] == 1]
    status = {'ready_second': float(ready['input_samples'][0] / RATE) if len(ready) else None,
              'accepted_frames': int(frames['calibration_frames'][-1]),
              'rejected_frames': int(frames['rejected_frames'][-1]) if 'rejected_frames' in frames.dtype.names else None,
              'clipped_samples': int(np.count_nonzero((actual <= -1) | (actual >= 32767/32768)))}
    if estimate.exists():
        status['profile_total_power'] = float(np.frombuffer(estimate.read_bytes(), dtype='<f4').sum())
    return actual, frames, status


def validate(args):
    args.output_dir.mkdir(parents=True, exist_ok=False)
    subprocess.run(['cargo', 'build', '--locked', '--release', '--example', 'denoise_pcm',
                    '--target', HOST], cwd=ROOT, check=True)
    after = ROOT / 'target' / HOST / 'release/examples/denoise_pcm.exe'
    before = args.before_executable.resolve()
    rng = np.random.default_rng(20261110)
    noise = rng.normal(0, 0.003, RATE * 20)
    if args.speech_wav:
        voice = read_speech(args.speech_wav)
        voice = np.tile(voice, int(np.ceil(len(noise)/len(voice))))[:len(noise)]
    else:
        t = np.arange(len(noise)) / RATE
        voice = sum(np.sin(2*np.pi*187*h*t)/h for h in range(1,16))
        voice *= 0.5 + 0.5 * np.sin(2*np.pi*2.7*t)**2
    voice = voice * (0.03 / rms(voice))
    cases = {'quiet': noise[:16*RATE].copy(),
             'speech-at-start': noise[:16*RATE].copy(),
             'speech-after-quiet': noise[:16*RATE].copy(),
             'continuous-speech': noise[:16*RATE] + voice[:16*RATE]}
    cases['speech-at-start'][:6*RATE] += voice[:6*RATE]
    cases['speech-after-quiet'][2*RATE:6*RATE] += voice[:4*RATE]
    if args.room_noise_wav:
        room = read_speech(args.room_noise_wav)[RATE//4:RATE+RATE//4]
        cases['recorded-room-quiet'] = np.tile(room, 16)
    results = {}
    checks = {}
    for name, samples in cases.items():
        directory = args.output_dir / name
        directory.mkdir()
        write_wav(directory/'input.wav', samples)
        item = {}
        for label, executable in [('before',before),('after',after)]:
            _, _, item[label] = run(executable, directory, label, samples)
        item['input_sha256'] = hashlib.sha256((directory/'input.s16le').read_bytes()).hexdigest()
        results[name] = item
        checks[name+'_no_clipping'] = item['after']['clipped_samples'] == 0
        if name in ('quiet','recorded-room-quiet'):
            ready = item['after']['ready_second']
            checks[name+'_completes'] = ready is not None and ready < 8
        if name in ('speech-at-start','speech-after-quiet'):
            ready = item['after']['ready_second']
            checks[name+'_waits_for_quiet'] = ready is not None and ready > 6
            old_power = item['before'].get('profile_total_power', 0)
            new_power = item['after'].get('profile_total_power', old_power)
            item['profile_power_after_vs_before'] = new_power / old_power
            checks[name+'_less_contamination'] = new_power < old_power * 0.3
    directory = args.output_dir / 'weak-speech'
    checks['continuous_speech_stays_unready'] = results['continuous-speech']['after']['ready_second'] is None
    quiet_power = results['quiet']['after']['profile_total_power']
    for name in ('speech-at-start', 'speech-after-quiet'):
        power = results[name]['after'].get('profile_total_power', float('inf'))
        checks[name+'_profile_matches_background'] = 0.75 < power / quiet_power < 1.5
    directory.mkdir()
    samples = noise.copy()
    clean = np.zeros_like(samples)
    clean[5*RATE:15*RATE] = voice[:10*RATE] * (0.0015 / rms(voice))
    samples += clean
    profile = noise_profile(rng.normal(0,0.003,RATE*5),(0.5,4.5))
    weak = {}
    outputs = {}
    for label, executable in [('before',before),('after',after)]:
        output, trace, status = run(executable,directory,label,samples,profile,True)
        active = trace[(trace['input_samples']>=5.5*RATE)&(trace['input_samples']<15*RATE)]
        status['activity_region_update_hops'] = int(np.count_nonzero(active['updating']))
        status['activity_region_guard_fraction'] = float(np.mean(active['speech_guard']))
        status['known_signal_rms_error'] = rms(output[6*RATE:14*RATE]-clean[6*RATE:14*RATE])
        outputs[label] = output
        weak[label] = status
    checks['weak_speech_less_learning'] = weak['after']['activity_region_update_hops'] < weak['before']['activity_region_update_hops']
    checks['weak_speech_error_not_worse'] = weak['after']['known_signal_rms_error'] < 1.06*weak['before']['known_signal_rms_error']
    checks['weak_speech_no_clipping'] = weak['after']['clipped_samples'] == 0
    results['weak-speech'] = weak
    # The supplied-profile fixed algorithm must retain exact output behavior.
    fixed = {}
    for label, executable in [('before',before),('after',after)]:
        fixed[label], _, _ = run(executable,directory,label+'-fixed',samples,profile,False)
    checks['fixed_profile_output_unchanged'] = bool(np.array_equal(fixed['before'],fixed['after']))
    report = {'results':results,'checks':checks,'passed':all(checks.values()),
              'before_executable_sha256':hashlib.sha256(before.read_bytes()).hexdigest(),
              'after_executable_sha256':hashlib.sha256(after.read_bytes()).hexdigest(),
              'speech_source_sha256':hashlib.sha256(args.speech_wav.read_bytes()).hexdigest() if args.speech_wav else None,
              'limitations':['No playback or on-board test in this tool.',
                             'Synthetic speech/harmonics do not establish human quiet-speech intelligibility.',
                             'Conservative spectral guards can reject tonal noise and cannot identify every low-SNR consonant.']}
    (args.output_dir/'report.json').write_text(json.dumps(report,indent=2,allow_nan=False)+'\n',encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir',type=Path,required=True)
    parser.add_argument('--before-executable',type=Path,required=True)
    parser.add_argument('--speech-wav',type=Path)
    parser.add_argument('--room-noise-wav',type=Path)
    arguments = parser.parse_args()
    result = validate(arguments)
    print(json.dumps(result,indent=2))
    if not result['passed']:
        raise SystemExit(1)
