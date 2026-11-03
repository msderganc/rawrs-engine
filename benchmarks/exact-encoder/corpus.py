#!/usr/bin/env python3
"""Regenerate exactly seven frozen inputs from public d466 source and fixed seeds."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent / 'vendor'))
import fixtures
ROOT = Path(__file__).resolve().parent
BASE = 'd4664689177ad50c5b94906e882e4a36fd3aca8f'

def generate(repository, output):
    expected = json.loads((ROOT / 'corpus.json').read_text())
    output.mkdir(parents=True, exist_ok=False)
    def git(*args):
        return subprocess.check_output(['git', '-C', str(repository), *args])
    names = ['crates/rars/src/codec/match_finder.rs', 'crates/rars/src/codec/rar50/encoder.rs']
    source = b''.join(b'\n// Frozen source: ' + n.encode() + b'\n' + git('show', BASE + ':' + n) for n in names)
    source = (source * (1 + 131072 // len(source)))[:131072]
    logs = b''.join(('2026-01-01T00:00:%02dZ request=%06d status=%d path=/api/records/%04d bytes=%d\n' %
                    (i % 60, i, 200 if i % 23 else 404, i % 997, (i * 37) % 65536)).encode() for i in range(3000))[:131072]
    with tempfile.TemporaryDirectory() as temporary:
        generated = Path(temporary) / 'fixtures'; fixtures.generate(generated)
        def f(name): return (generated / 'hybrid' / name).read_bytes()
        payloads = {'source-and-logs.txt': source + logs,
            'related-releases.jsonl': f('releases/v1/export.jsonl') + f('releases/v2/export.jsonl'),
            'shifted-repeat.bin': f('shifted/base.bin') + f('shifted/with-prefix.bin'),
            'dictionary-distance.bin': f('distance/long-gap.bin'),
            'unrelated-data.bin': f('unrelated/first.bin') + f('unrelated/second.bin')}
    pressure = bytearray(b''.join(b'ANCH' + fixtures.noise(252, 0xAC000001 + i) for i in range(768)))
    first = bytes(pressure[:1024])
    for offset in (66048, 140288): pressure[offset:offset + 1024] = first
    payloads['hot-prefix-pressure.bin'] = pressure
    blobs = []
    for entry in git('ls-tree', '-rz', BASE).split(b'\0'):
        if not entry: continue
        metadata, path = entry.split(b'\t', 1); mode, kind, object_id = metadata.decode().split()
        if kind == 'blob' and mode in ('100644', '100755') and path.endswith(b'.rs'):
            blobs.append((path.decode(), object_id))
    result, seen, recipe = bytearray(), set(), []
    for path, object_id in sorted(blobs):
        if object_id in seen: continue
        seen.add(object_id); data = git('cat-file', 'blob', object_id)
        marker = ('\n// Frozen rars source path: ' + path + '\n').encode()
        piece = (marker + data)[:2097152 - len(result)]; result.extend(piece)
        recipe.append({'path': path, 'git_blob': object_id, 'sha256': hashlib.sha256(data).hexdigest(), 'included_bytes': len(piece)})
        if len(result) == 2097152: break
    payloads['source-tree-2m.rs'] = result
    for record in expected['files']:
        data = payloads[record['path']]
        assert len(data) == record['size'] and hashlib.sha256(data).hexdigest() == record['sha256'], record['path']
        path = output / record['path']; path.write_bytes(data); path.chmod(0o644); os.utime(path, (1700000000, 1700000000))
    (output / 'manifest.json').write_text(json.dumps(expected, sort_keys=True, indent=2) + '\n')
    (output / 'source-recipe.json').write_text(json.dumps({'revision': BASE, 'distinct_blobs': recipe, 'no_tiling': True}, indent=2) + '\n')
    output.chmod(0o755); os.utime(output, (1700000000, 1700000000))
    return output / 'manifest.json'

if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__); p.add_argument('--repository', type=Path, required=True); p.add_argument('--output', type=Path, required=True)
    a = p.parse_args(); print(generate(a.repository.resolve(), a.output.resolve()))
