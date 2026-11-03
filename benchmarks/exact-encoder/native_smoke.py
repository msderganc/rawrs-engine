#!/usr/bin/env python3
"""Twelve correctness-only candidate cases; never builds or edits source.

Usage: native_smoke.py LABEL CANDIDATE --source-revision FULL_SHA
       --native-rar BINARY --corpus DIRECTORY --output DIRECTORY
Output directories are never reused. Supply the frozen source revision; matching
local frozen manifests are also checked when available.
Every supported archive is integrity-tested and extracted by native RAR and by
the candidate. No timing, throughput or performance qualification is produced.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess

ROOT = Path(__file__).resolve().parent
FIXED_TIME = 1700000000
LIMIT = 268435456


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for part in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(part)
    return digest.hexdigest()


def read_json(path):
    return json.loads(path.read_text())


def write_json(path, value):
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, sort_keys=True, indent=2) + '\n')
    os.replace(temporary, path)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def source_identity(binary_hash, supplied):
    matches = []
    for path in sorted(ROOT.glob('run-*/manifest.json')):
        manifest = read_json(path)
        if manifest['binaries']['candidate']['sha256'] != binary_hash:
            continue
        require(sha(path) == (path.parent / 'manifest.sha256').read_text().strip(),
                'Matching frozen source manifest hash changed')
        matches.append({'source': manifest['source'], 'manifest': str(path), 'manifest_sha256': sha(path)})
    revisions = {record['source']['revision'] for record in matches}
    if supplied:
        require(re.fullmatch(r'[0-9a-fA-F]{40}', supplied), '--source-revision must be a full commit SHA')
        supplied = supplied.lower()
        require(not revisions or revisions == {supplied}, 'Supplied source revision differs from frozen binary provenance')
        return {'revision': supplied, 'evidence': matches,
                'scope': 'matching frozen binary/source manifest' if matches else 'operator-supplied source revision; binary build not independently attested'}
    require(len(revisions) == 1, 'No unique frozen candidate source revision; supply --source-revision')
    return {'revision': next(iter(revisions)), 'evidence': matches, 'scope': 'matching frozen binary/source manifest'}


def execute(command, cwd, area, label, environment):
    command = list(map(str, command))
    stdout, stderr = area / (label + '.stdout'), area / (label + '.stderr')
    require(not stdout.exists() and not stderr.exists(), 'Never overwrite command logs')
    with stdout.open('xb') as out, stderr.open('xb') as err:
        try:
            completed = subprocess.run(command, cwd=cwd, env=environment, stdout=out, stderr=err, timeout=120)
            record = {'command': command, 'exit': completed.returncode, 'timeout': False,
                      'stdout': stdout.name, 'stderr': stderr.name}
        except subprocess.TimeoutExpired:
            record = {'command': command, 'exit': None, 'timeout': True,
                      'stdout': stdout.name, 'stderr': stderr.name}
    write_json(area / (label + '.json'), record)
    return record


def successful(command, cwd, area, label, environment):
    record = execute(command, cwd, area, label, environment)
    require(record['exit'] == 0 and not record['timeout'], label + ': command failed; logs retained')
    return record


def verify_tree(target, names, payloads):
    actual = {}
    for path in target.rglob('*'):
        metadata = path.lstat()
        require(stat.S_ISREG(metadata.st_mode), 'Unexpected directory, link or special output: ' + str(path))
        actual[path.relative_to(target).as_posix()] = path
    require(set(actual) == set(names), 'Extracted member paths differ')
    records = {}
    for name in names:
        path = actual[name]
        metadata = path.stat()
        contents = path.read_bytes()
        require(contents == payloads[name], 'Extracted bytes differ: ' + name)
        require(stat.S_IMODE(metadata.st_mode) == 0o644, 'Extracted permissions differ: ' + name)
        require(int(metadata.st_mtime) == FIXED_TIME, 'Extracted mtime differs: ' + name)
        records[name] = {'bytes': len(contents), 'sha256': hashlib.sha256(contents).hexdigest(),
                         'mode': 0o644, 'mtime': FIXED_TIME, 'type': 'file'}
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('label')
    parser.add_argument('candidate', type=Path)
    parser.add_argument('--source-revision', required=True)
    parser.add_argument('--native-rar', type=Path, required=True)
    parser.add_argument('--corpus', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    arguments = parser.parse_args()
    require(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*', arguments.label), 'Unsafe output label')
    global RAR, ROOT
    RAR = arguments.native_rar.resolve(strict=True)
    ROOT = arguments.output.resolve()
    ROOT.mkdir(parents=True, exist_ok=True)
    binary = arguments.candidate.resolve(strict=True)
    binary_hash, native_hash = sha(binary), sha(RAR)
    identity = source_identity(binary_hash, arguments.source_revision)
    area = ROOT / ('native-smoke-' + arguments.label)
    summary_path = ROOT / ('native-smoke-' + arguments.label + '.json')
    require(not area.exists() and not summary_path.exists(), 'Use a fresh label; never replace prior smoke evidence')
    source_payload = arguments.corpus.resolve() / 'source-tree-2m.rs'
    contents = source_payload.read_bytes()
    source_manifest = read_json(arguments.corpus.resolve() / 'manifest.json')
    expected_source = next(record for record in source_manifest['files'] if record['path'] == source_payload.name)
    require(len(contents) == 2097152 and sha(source_payload) == expected_source['sha256'],
            'Expected the unchanged full 2 MiB source fixture')
    payloads = {'plain.bin': contents, 'delta.bin': contents,
                'first.bin': contents[:1048576], 'second.bin': contents[1048576:]}
    # The earlier current-rars smoke qualified these CLI combinations. Any
    # nonzero encode exit here is therefore unexpected, unless a prior baseline
    # record explicitly identifies that same combination as unsupported.
    baseline_cases = {}  # All twelve fixed combinations must pass; refusals fail.
    area.mkdir()
    source = area / 'input'
    source.mkdir()
    for name, data in payloads.items():
        path = source / name
        path.write_bytes(data)
        path.chmod(0o644)
        os.utime(path, (FIXED_TIME, FIXED_TIME))
    source.chmod(0o755)
    os.utime(source, (FIXED_TIME, FIXED_TIME))
    environment = {**os.environ, 'LC_ALL': 'C', 'TZ': 'UTC', 'RAYON_NUM_THREADS': '1'}
    for key in list(environment):
        if key.startswith('RARS_'):
            del environment[key]
    proof = {'scope': 'Correctness only; no timed benchmark or performance promotion',
             'candidate': str(binary), 'candidate_sha256': binary_hash, 'source': identity,
             'native_rar': str(RAR), 'native_rar_sha256': native_hash,
             'helper_sha256': sha(Path(__file__)),
             'source_fixture': {'path': str(source_payload), 'bytes': len(contents), 'sha256': sha(source_payload)},
             'inherited_qualification': None,
             'expected_case_count': 12, 'cases': [], 'status': 'incomplete',
             'verification_scope': 'Exact file bytes and member paths, POSIX permissions, whole-second mtime; owner/group, xattrs and ACLs are not compared.'}
    write_json(summary_path, proof)
    try:
        for format_name in ('rar50', 'rar70'):
            dictionary = 4194304 if format_name == 'rar50' else 3145728
            for level in (1, 3):
                for case, names, extra in (
                    ('plain', ['plain.bin'], []),
                    ('delta', ['delta.bin'], ['--delta-filter', '1']),
                    ('solid', ['first.bin', 'second.bin'], ['--solid']),
                ):
                    label = f'{format_name}-level{level}-{case}'
                    case_area = area / label
                    case_area.mkdir()
                    archive = case_area / 'candidate.rar'
                    record = {'format': format_name, 'level': level, 'case': case,
                              'dictionary_bytes': dictionary, 'members': names, 'status': 'incomplete'}
                    proof['cases'].append(record)
                    write_json(summary_path, proof)
                    command = [binary, '--threads', '1', '--progress', 'never', 'a',
                               '--format', format_name, '--level', str(level), '--dict-size', str(dictionary),
                               '--memory-limit', str(LIMIT), '--max-memory', str(LIMIT),
                               '--no-filter', *extra, archive, *names]
                    encoded = execute(command, source, case_area, 'encode', environment)
                    record['encode'] = encoded
                    if encoded['exit'] != 0 or encoded['timeout']:
                        previous = baseline_cases.get((format_name, level, case), {})
                        qualified_refusal = (previous.get('status') in ('refused', 'unsupported')
                                             and previous.get('baseline_refusal_qualified') is True
                                             and not encoded['timeout'])
                        record['status'] = 'refused'
                        record['classification'] = 'inherited-qualified-refusal' if qualified_refusal else 'unexpected-encode-refusal'
                        record['baseline_qualification'] = previous
                        write_json(case_area / 'result.json', record)
                        write_json(summary_path, proof)
                        if qualified_refusal:
                            continue
                        raise RuntimeError(label + ': unexpected encoder refusal; not a passing case')
                    record['archive_sha256'] = sha(archive)
                    record['archive_bytes'] = archive.stat().st_size
                    successful([RAR, 'lt', '-p-', archive], area, case_area, 'native-actual-metadata', environment)
                    record['actual_native_archive_metadata'] = str(case_area / 'native-actual-metadata.stdout')
                    record['format_scope'] = 'Native listing retained verbatim; actual compression version is not inferred solely from the requested target name.'
                    record['decoders'] = {}
                    for decoder, extractor in (('native', RAR), ('candidate', binary)):
                        target = case_area / ('extracted-' + decoder)
                        target.mkdir()
                        if decoder == 'native':
                            test = [extractor, 't', '-idq', '-p-', '-mt1', archive]
                            extract = [extractor, 'x', '-idq', '-y', '-p-', '-mt1', archive, str(target) + '/']
                        else:
                            test = [extractor, '--threads', '1', 'test', archive]
                            extract = [extractor, '--threads', '1', 'extract', archive, target]
                        tested = successful(test, area, case_area, decoder + '-integrity', environment)
                        decoded = successful(extract, area, case_area, decoder + '-extract', environment)
                        members = verify_tree(target, names, payloads)
                        members_path = case_area / (decoder + '-members.json')
                        write_json(members_path, members)
                        record['decoders'][decoder] = {'integrity': tested, 'extract': decoded,
                            'members_path': str(members_path), 'members_sha256': sha(members_path),
                            'contents_paths_mode_mtime': 'pass'}
                    require(sha(archive) == record['archive_sha256'], 'Archive changed during extraction')
                    record['status'] = 'pass'
                    write_json(case_area / 'result.json', record)
                    write_json(summary_path, proof)
        require(sha(binary) == binary_hash and sha(RAR) == native_hash, 'A supplied executable changed during smoke verification')
        passed = sum(record['status'] == 'pass' for record in proof['cases'])
        refused = sum(record['status'] == 'refused' for record in proof['cases'])
        require(len(proof['cases']) == 12 and passed + refused == 12, 'Incomplete smoke coverage')
        proof['passed_cases'] = passed
        proof['inherited_refusals'] = refused
        proof['all_twelve_passed'] = passed == 12
        proof['status'] = 'pass' if passed == 12 else 'complete-with-explicit-inherited-refusals'
        write_json(summary_path, proof)
        print(f'{passed}/12 cases passed both native and candidate test/extraction; {refused} explicitly inherited refusals.')
        print(str(summary_path))
    except BaseException as error:
        proof['status'] = 'failed'
        proof['error'] = str(error)
        write_json(summary_path, proof)
        raise


if __name__ == '__main__':
    try:
        main()
    except (OSError, KeyError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        raise SystemExit('Correctness smoke stopped; evidence retained: ' + str(error))
