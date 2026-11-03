#!/usr/bin/env python3
"""Analyze completed frozen local runs. Never launches encoders or modifies sources."""
from pathlib import Path
import hashlib
import importlib.util
import json
import math
import os
import random
import statistics

BOOTSTRAP_RESAMPLES = 5000


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def read_json(path):
    return json.loads(path.read_text())


def load_run(run, label):
    script = run / 'frozen/hybrid_candidates.py'
    spec = importlib.util.spec_from_file_location('frozen_runner_' + label, script)
    runner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runner)
    manifest = runner.load(run)
    require(manifest.get('experiment') == 'controls', f'{label}: controls experiment required')
    require(len(manifest['configurations']) == 1, f'{label}: exactly one frozen configuration required')
    require(manifest['confirmation_repetitions'] == 15, f'{label}: expected 15 paired measured repetitions')
    journal = runner.journal(run)
    events = runner.latest(journal)
    require(all(row['event'] == 'complete' for row in events.values()),
            f'{label}: retained unfinished, failed or excluded tasks; no complete result claim')
    configuration = manifest['configurations'][0]['id']
    expected = runner.tasks(manifest, 'screen') + runner.tasks(manifest, 'confirm', [configuration])
    require(set(events) == {runner.task_id(task) for task in expected},
            f'{label}: incomplete frozen screening/warmup/confirmation coverage')
    all_rows = runner.verified_results(run, journal, 'screen') + runner.verified_results(run, journal, 'confirm')
    require(len(all_rows) == len(expected), f'{label}: missing verified results')
    measured = [row for row in all_rows
                if row['task']['phase'] == 'confirm' and row['task']['repetition'] >= 0]
    case_ids = {case['id'] for case in manifest['cases']}
    require({row['task']['case'] for row in measured} == case_ids, f'{label}: missing measured workload')
    repetitions = manifest['confirmation_repetitions']
    for case_id in case_ids:
        selected = [row for row in measured if row['task']['case'] == case_id]
        require(len(selected) == repetitions and {row['task']['repetition'] for row in selected} == set(range(repetitions)),
                f'{label}/{case_id}: require exactly {repetitions} measured pairs')
        require(len({row['roles']['candidate']['archive_sha256'] for row in selected}) == 1,
                f'{label}/{case_id}: nondeterministic candidate archive bytes')
    return runner, manifest, all_rows, measured


def bootstrap(values, label):
    require(bool(values) and all(math.isfinite(value) and value > 0 for value in values),
            f'{label}: missing/nonpositive metric; cannot claim a ratio')
    rng = random.Random(int(hashlib.sha256(label.encode()).hexdigest(), 16))
    medians = sorted(statistics.median(rng.choices(values, k=len(values)))
                     for _ in range(BOOTSTRAP_RESAMPLES))
    return {'median_ratio': statistics.median(values),
            'bootstrap_95pct': [medians[int(.025 * len(medians))], medians[int(.975 * len(medians))]],
            'paired_ratios': values}


def role_metrics(rows, role):
    def median(*path):
        values = []
        for row in rows:
            value = row['roles'][role]
            for key in path:
                value = value[key]
            if value is not None:
                values.append(value)
        return statistics.median(values) if values else None
    return {'archive_bytes': median('archive_bytes'),
            'encode_wall_seconds': median('encode', 'wall_seconds'),
            'encode_cpu_seconds': median('encode', 'cpu_seconds'),
            'encode_peak_rss_bytes': median('encode', 'peak_rss_bytes'),
            'native_decode_wall_seconds': median('decoders', 'native', 'extract', 'wall_seconds'),
            'native_decode_peak_rss_bytes': median('decoders', 'native', 'extract', 'peak_rss_bytes'),
            'rars_decode_wall_seconds': median('decoders', 'candidate', 'extract', 'wall_seconds'),
            'rars_decode_peak_rss_bytes': median('decoders', 'candidate', 'extract', 'peak_rss_bytes')}


def verification_counts(runner, run, manifest, rows, label):
    cases = {case['id']: case for case in manifest['cases']}
    counts = {'verified_tasks': len(rows), 'candidate_archives_native_extracted': 0,
              'all_role_archives_native_extracted': 0, 'all_role_archives_rars_extracted': 0,
              'verified_archives': 0}
    for row in rows:
        case = cases[row['task']['case']]
        area = run / 'tasks' / runner.task_id(row['task'])
        require(set(row['roles']) == set(manifest['sample_roles']), f'{label}: unexpected/missing sample role')
        for role, data in row['roles'].items():
            require(runner.sha(area / (role + '.rar')) == data['archive_sha256'],
                    f'{label}: retained archive hash mismatch')
            require(set(data['decoders']) == {'native', 'candidate'}, f'{label}: both decoder receipts required')
            counts['verified_archives'] += 1
            for decoder, decoded in data['decoders'].items():
                proof = runner.read_json(area / decoded['verification_receipt'])
                require(proof['verified'] and proof['task_id'] == runner.task_id(row['task']),
                        f'{label}: invalid extraction proof')
                require(proof['expected_tree_sha256'] == runner.digest(case['members']),
                        f'{label}: extraction proof is for a different member tree')
                runner.validate_tree(runner.read_json(area / proof['members_json']), case['members'])
                counts['all_role_archives_native_extracted' if decoder == 'native'
                       else 'all_role_archives_rars_extracted'] += 1
                if role == 'candidate' and decoder == 'native':
                    counts['candidate_archives_native_extracted'] += 1
    counts['contents_paths_mode_mtime'] = 'pass'
    return counts


def settings(manifest):
    return {'experiment': manifest['experiment'], 'profiles': manifest['profiles'],
            'fixed_dictionary_bytes': manifest.get('fixed_dictionary_bytes'),
            'execution': manifest['execution'], 'worker_options': manifest['worker_options'],
            'confirmation_repetitions': manifest['confirmation_repetitions'],
            'workload_count': len(manifest['cases']),
            'total_input_bytes': sum(member.get('bytes', 0) for case in manifest['cases']
                                     for member in case['members'].values()),
            'per_case_control_settings': {case['id']: case['control_settings'] for case in manifest['cases']},
            'metadata_scope': manifest['metadata_scope'], 'cold_scope': manifest['cold_scope'],
            'budgets': manifest['budgets'], 'task_timeout_seconds': manifest['task_timeout_seconds']}

