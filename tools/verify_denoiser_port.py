"""Compare exact Rust firmware DSP with the accepted Python offline output."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

import numpy as np

from denoise_offline import read_wav, write_wav

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--reference', type=Path, default=Path('captures/denoise-offline-v1'))
parser.add_argument('--output-dir', type=Path, default=Path('captures/denoiser-port'))
args = parser.parse_args()
args.output_dir.mkdir(parents=True, exist_ok=True)
original = read_wav(args.reference / 'original.wav')
reference = read_wav(args.reference / 'wiener-12db.wav')
profile = np.load(args.reference / 'noise-power.npy', allow_pickle=False).astype('<f4')
raw = args.output_dir / 'input.s16le'
output = args.output_dir / 'rust.s16le'
noise = args.output_dir / 'noise.f32le'
raw.write_bytes(np.rint(original * 32768).astype('<i2').tobytes())
noise.write_bytes(profile.tobytes())
subprocess.run(['cargo', 'run', '--locked', '--release', '--example', 'denoise_pcm',
                '--target', 'x86_64-pc-windows-msvc', '--', str(raw), str(output), str(noise)], check=True)
actual = np.frombuffer(output.read_bytes(), dtype='<i2').astype(np.float64) / 32768
if len(actual) != len(reference):
    raise ValueError('Rust output length differs from offline reference')
error = (actual - reference) * 32768
report = {'input_sha256': hashlib.sha256((args.reference / 'original.wav').read_bytes()).hexdigest(),
          'reference': str(args.reference / 'wiener-12db.wav'),
          'samples': len(actual), 'max_abs_error_lsb': float(np.max(np.abs(error))),
          'rms_error_lsb': float(np.sqrt(np.mean(error ** 2))),
          'same_samples_fraction': float(np.mean(error == 0)),
          'passed': bool(np.max(np.abs(error)) <= 2),
          'scope': 'Host execution of exact no_std f32 DSP with the same fixed profile; startup on-device calibration is separate.'}
write_wav(args.output_dir / 'rust-12db.wav', actual)
Path('artifacts/denoiser-host-parity.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
print(json.dumps(report, indent=2))
if not report['passed']:
    raise SystemExit('Rust/Python comparison exceeds 2 LSB tolerance')
