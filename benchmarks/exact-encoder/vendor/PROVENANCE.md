# Vendored benchmark sources

`worker.py` is copied unchanged from the original mechanical runner `scripts/hybrid_candidates.py`, SHA256 `866235a79a0b6e0bde6873efcb17613219a4736c5e74913c26b1611bd880771d`. `fixtures.py` is the deterministic fixture generator, SHA256 `02477c86de136d4c9a9eabb2839f08c69bf3a6d55859b523d9c854bfa48c720e`. Both are rars source under the repository's license. No network fetch or unmerged PR dependency is required.

The outer command exposes only the fixed ordinary-controls comparison. The vendored worker is retained unchanged so its role ordering, resource checks, subprocess CPU accounting and journal verification remain auditable. Its broader historical command surface is not a request to run additional experiments.
