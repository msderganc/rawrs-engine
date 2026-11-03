#!/usr/bin/env python3
"""Correctness-only package checks; no encoders, compilation or timed jobs."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import analysis
import build_controls
import corpus
ROOT=Path(__file__).resolve().parent

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--repository',type=Path,required=True);a=p.parse_args()
    assert hashlib.sha256((ROOT/'vendor/worker.py').read_bytes()).hexdigest()=='866235a79a0b6e0bde6873efcb17613219a4736c5e74913c26b1611bd880771d'
    with tempfile.TemporaryDirectory() as temp:
        directory=Path(temp);manifest=corpus.generate(a.repository.resolve(),directory/'corpus');m=json.loads(manifest.read_text())
        assert len(m['files'])==7 and sum(f['size'] for f in m['files'])==2806768
        for f in m['files']:
            path=manifest.parent/f['path'];assert path.stat().st_mode&0o777==0o644 and int(path.stat().st_mtime)==1700000000
        receipt=build_controls.prepare(a.repository.resolve(),directory/'references',True)
        assert set(receipt['controls'])=={'current','wide','cyclic'}
    # Preserve the original percentile-bootstrap implementation and exact bound.
    assert analysis.bootstrap([1.0]*15,'known')['bootstrap_95pct']==[1.0,1.0]
    assert analysis.bootstrap([1.001]*15,'known')['bootstrap_95pct'][1]>1
    print('PASS: exact seven-case bytes/metadata; both reference patch trees; worker SHA; unchanged bootstrap boundaries. No benchmark/build ran.')
if __name__=='__main__':main()
