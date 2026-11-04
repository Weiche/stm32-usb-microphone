"""Summarize observable cumulative errors; does not replace USB bus analysis."""
import argparse
import json
import re
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('log', type=Path)
parser.add_argument('--json', type=Path, required=True)
parser.add_argument('--expect-clean', action='store_true')
args = parser.parse_args()
text = args.log.read_text(encoding='utf-8')
rows = []
for line in text.splitlines():
    values = {key: int(value) for key, value in re.findall(r'\b([a-z0-9_]+)=(\d+)\b', line)}
    if values:
        rows.append(values)
keys = ['packets', 'usb_errors', 'disabled', 'discarded_samples', 'ring_overruns',
        'dma_errors', 'overflow_events', 'lost_samples', 'underruns', 'missing_samples',
        'priming_packets', 'reset_discarded', 'clipped', 'decode_max_us', 'prepare_max_us']
maxima = {key: max(row[key] for row in rows if key in row) for key in keys
          if any(key in row for row in rows)}
required = ['usb_errors', 'ring_overruns', 'dma_errors', 'overflow_events',
            'lost_samples', 'underruns', 'missing_samples']
clean = all(key in maxima and maxima[key] == 0 for key in required)
report = {'log': str(args.log), 'max_observed_cumulative_values': maxima,
          'observed_error_counters_zero': clean,
          'stream_starts': text.count('usb interface=1 alt=1'),
          'usb_resets': text.count('usb reset'),
          'scope': 'Only visible firmware counters. Normal alt=0 cancels one pending packet; '
                   'discarded_samples includes this. No proof of absence of every on-wire loss.'}
args.json.write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
print(json.dumps(report, indent=2))
if args.expect_clean and not clean:
    raise SystemExit(1)
