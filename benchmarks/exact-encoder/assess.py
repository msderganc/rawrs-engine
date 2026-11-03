#!/usr/bin/env python3
"""Read an existing completed qualification; apply the original unrounded bounds."""
import hashlib
import json
from pathlib import Path
import sys

path = Path(sys.argv[1]).resolve(strict=True)
q = json.loads(path.read_text())
assert len(q['workloads']) == 7 and q['exact_current_archive_identity_all_workloads']
assert all(w['measured_pairs_per_comparison'] == 15 for w in q['workloads'].values())
assert q['settings']['fixed_dictionary_bytes'] == 4194304
assert q['settings']['worker_options'] == {'rars_add': ['--no-filter'], 'native_add': ['-mc-']}
assert q['settings']['execution'] == {'confirm_threads': 1, 'parallel_screening': False, 'screen_threads': 1}
for run in ('current_run', 'prior_run', 'cyclic_run'):
    v = q[run]['verification']
    assert v['verified_tasks'] == 119 and v['all_role_archives_native_extracted'] == 357
    assert v['all_role_archives_rars_extracted'] == 357 and v['contents_paths_mode_mtime'] == 'pass'
criteria = [('dictionary-distance', 'candidate_current', 'cpu_seconds'),
            ('hot-prefix-pressure', 'candidate_current', 'cpu_seconds'),
            ('source-tree-2m', 'candidate_prior', 'wall_seconds'),
            ('source-tree-2m', 'candidate_prior', 'cpu_seconds'),
            ('source-tree-2m', 'candidate_cyclic', 'peak_rss_bytes')]
checks = []
for case, control, metric in criteria:
    record = q['workloads'][case][control][metric]
    assert len(record['paired_ratios']) == 15
    upper = record['bootstrap_95pct'][1]
    checks.append({'case': case, 'comparison': control, 'metric': metric,
                   'median_ratio': record['median_ratio'], 'bootstrap_95pct': record['bootstrap_95pct'],
                   'required_upper_bound': 1.0, 'passed': upper <= 1.0})
passed = all(c['passed'] for c in checks)
print(json.dumps({'passed': passed, 'qualification': str(path),
                  'qualification_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                  'source': q['source'], 'checks': checks,
                  'scope': 'Original standalone criteria only; no reruns, rounding tolerance, batch substitution, default promotion or full-gate claim.'},
                 indent=2, sort_keys=True))
sys.exit(0 if passed else 2)
