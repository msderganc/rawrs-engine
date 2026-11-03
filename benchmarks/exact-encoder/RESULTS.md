# Exact encoder: fresh portable qualification

The clean implementation at `a34ab21c8054997a300c42783026ab0d0d1df8ef` passes **all five original standalone bounds** through the portable entrypoint, preserving all seven current-rars archive SHA256 values. Native compatibility smoke passes **12/12**; both full project gates **pass**. The encoder is an explicit `experimental-ring-state` option. These bounded local results do not establish general or native-RAR throughput parity.

## Fixed protocol and verification

RAR5, level 3, dictionary 4 MiB, filters off, one thread, non-solid input, 256 MiB controlled memory; seven inputs total 2,806,768 bytes. The unchanged worker freezes seed 20261017, role ordering and CPU accounting. Each of three controls has seven screens, seven warmups and 15 measured pairs/workload: **315 measured pairs, 357 tasks, and 1,071 archives extracted by each decoder**. Native RAR and candidate rars verify bytes, paths, POSIX modes and whole-second mtimes. [Portable protocol](README.md), [complete path-free export](results/qualification.json), [five-bound assessment](results/assessment.json).

The export is copied unchanged from the completed run. Ratios are paired medians; intervals use 5,000 deterministic bootstrap resamples. All five unrounded upper bounds must be <=1. No batch substitution or relaxed threshold is used.

## Five original bounds

| Criterion | Ratio [95% interval] | Unrounded upper bound | Verdict |
|---|---:|---:|---|
| dictionary-distance / current CPU | 0.880 [0.818, 0.920] | 0.9202565784678184 | PASS |
| hot-prefix-pressure / current CPU | 0.811 [0.771, 0.828] | 0.8276523629852274 | PASS |
| source-tree-2m / wide wall | 0.949 [0.943, 0.965] | 0.9651860558404237 | PASS |
| source-tree-2m / wide CPU | 0.947 [0.943, 0.965] | 0.9652270307559185 | PASS |
| source-tree-2m / cyclic RSS | 0.942 [0.940, 0.946] | 0.9455066921606119 | PASS |

Dictionary-distance and pressure use **12.0% and 18.9% less CPU** than current rars in these paired samples. On 2 MiB source, CPU/wide is **0.9471**, wall/wide **0.9488**, and RSS/cyclic **0.94174**. The exact source archive is 316,499 bytes, versus 359,439 for the cyclic control.

## Seven workloads against current rars

| Workload | Archive bytes | Wall ratio [95% interval] | CPU ratio [95% interval] | RSS ratio [95% interval] |
|---|---:|---:|---:|---:|
| dictionary-distance | 172,331 | 1.019 [0.912, 1.058] | 0.880 [0.818, 0.920] | 0.889 [0.888, 0.891] |
| hot-prefix-pressure | 192,801 | 0.993 [0.889, 1.053] | 0.811 [0.771, 0.828] | 0.894 [0.891, 0.895] |
| related-releases | 1,403 | 0.994 [0.964, 1.070] | 0.975 [0.887, 0.997] | 0.829 [0.825, 0.831] |
| shifted-repeat | 8,650 | 0.991 [0.900, 1.032] | 0.927 [0.889, 0.997] | 0.887 [0.882, 0.891] |
| source-and-logs | 35,101 | 0.901 [0.843, 0.926] | 0.880 [0.874, 0.899] | 0.862 [0.860, 0.865] |
| source-tree-2m | 316,499 | 0.608 [0.595, 0.645] | 0.598 [0.586, 0.645] | 0.618 [0.606, 0.629] |
| unrelated-data | 24,733 | 1.018 [0.974, 1.720] | 0.927 [0.859, 1.057] | 0.887 [0.885, 0.889] |

Intervals crossing 1 remain visible: several small-case wall results and unrelated-data CPU do not establish a win. Unrelated-data wall/wide is 1.030 [1.004, 1.121], an observed regression in that comparison. The five passed targets are not a blanket claim for every workload or metric.

## 2 MiB source: each fresh control

| Control | Wall ratio [95% interval] | CPU ratio [95% interval] | RSS ratio [95% interval] |
|---|---:|---:|---:|
| Current | 0.608 [0.595, 0.645] | 0.598 [0.586, 0.645] | 0.618 [0.606, 0.629] |
| Wide | 0.949 [0.943, 0.965] | 0.947 [0.943, 0.965] | 0.882 [0.879, 0.884] |
| Cyclic | 0.921 [0.916, 0.947] | 0.921 [0.918, 0.945] | 0.942 [0.940, 0.946] |

Control timings varied across local runs. In this current-control run, source-case CPU medians are 2.072 s for current and 1.278 s for the candidate; the paired ratio is not a ratio of these separate medians. No cross-run ratio or historical timing is substituted. Wide/cyclic use their own fresh paired candidate observations. Small wall times include startup and up-to-10-ms polling quantization; CPU includes per-process user/system startup cost. No cache eviction or cold selective extraction was performed. Owner/group, ACLs and xattrs are unmeasured.

## Control rebuild and compatibility proof

All three controls were actually rebuilt from public d466 plus the source-only reference patches; [build identities](results/rebuilt-controls.json) record their new hashes and compiler. A separate [seven-case screen against rebuilt current](results/rebuilt-control-screen.json) passed exact archive identity and 21 candidate/current/native archives through both decoders. Rebuilt wide/cyclic have build/source reconstruction proof; this screen is not their 15-pair performance qualification.

The strict run above deliberately uses the **original known frozen control binary hashes** below. Rebuilt binaries were not silently substituted; new builds are not claimed bit-identical across paths or machines. No proprietary executable or old research dump is included.

## Frozen compiler and binary identities

Recorded freezer/compiler: Rust 1.97.1 Homebrew (`8bab26f4f68e0e26f0bb7960be334d5b520ea452`), LLVM 22.1.8, `aarch64-apple-darwin`. Prebuilt-control compiler provenance is distinct from this freezer record; their identities are bound to the original known hashes and reference source manifest.

| Role | Source revision | Binary SHA256 |
|---|---|---|
| Candidate | `a34ab21c8054997a300c42783026ab0d0d1df8ef` | `8202a0de119ddfca1cbe4db985e23f0194558e08c9b0483cf0dd445d188417f6` |
| current | `d4664689177ad50c5b94906e882e4a36fd3aca8f` | `6c6a5ef616f69bb428933f1a6d42a86a2434d09477ec6339b12cbe8cdcf938dd` |
| wide | `c1d7fc8e90392074c15840a864bc88e119d607e8` | `d53c9b93c37c233751bbdeac7b080ec4429ed5cd289ba6ffbe535ef7d7361e60` |
| cyclic | `822681395d84a0523519b584d3a64ee8f7b370aa` | `e7e414370e696d7e214269a96fec2adbfbea6e59a7bfd68916cbcd783ce6de76` |

Source tree: `5bc6817b569605cb4ab26a0868b47cd5a74bb9b7`; clean dirty-diff hash: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`. Worker SHA256: `866235a79a0b6e0bde6873efcb17613219a4736c5e74913c26b1611bd880771d`. Full frozen control-manifest hashes, paired observations, archive hashes and verification counts remain in the exported JSON.

## Final validation

- Native [12-case smoke](results/native-smoke.json): **12/12 pass, zero refusals**, tied to the same candidate SHA. RAR5/4 MiB and RAR7/fractional 3 MiB; levels 1/3; plain, delta-1 and two-member solid; integrity tests and extraction by both native and candidate decoders, with full 2 MiB payloads.
- Fresh [validation receipts](results/validation.json): both full project gates **pass**. Encoder gate: 2,900 Rust passes (17 ignored), 728 Python passes (8 filesystem capability skips), WASM/npm build and all checks pass. The separate experimental-feature library run passes 1,719 tests (3 ignored), and strict all-feature Clippy passes. The exact new write-only CI command passes 1,473 tests (3 ignored).
- Production code and benchmark implementation remain identical to the measured revision. Subsequent changes add CI coverage, documentation and result receipts only.
- No default promotion, installation or native-RAR performance parity claim.
