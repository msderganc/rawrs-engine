# Default RAR5/7 encoder memory coverage

Deterministic memory-policy tests and optional process-RSS observations serve
different purposes. RSS depends on the allocator, operating system, process
startup, and scheduling. It is never a correctness threshold in this package.

## Deterministic managed-allocation checks

`crates/rars/tests/encoder_memory_regression.rs` runs with the default encoder,
without the compatibility feature alias. It uses the public `Builder::write_to`
entry point for fitted default dictionaries and the public streaming writer
with `WriterOptions` for explicit dictionaries. CI also runs it independently
with `write` and `write,parallel`:

```sh
cargo test -p rars --locked --test encoder_memory_regression
cargo test -p rars --locked --no-default-features --features write --test encoder_memory_regression
cargo test -p rars --locked --no-default-features --features write,parallel --test encoder_memory_regression
```

| Cases | Inputs | Dictionaries | Levels | Threads | Managed ceiling |
| --- | --- | --- | --- | --- | --- |
| RAR5 and RAR7 ordinary writes | Two identical 160 KiB members, fixed-seed noise or repeated text | Builder-fitted 256 KiB, explicit 128 KiB and 1 MiB; RAR7 also 192 KiB | 1, 3, 5 | 1 and 2 when parallel is enabled | 96 MiB |
| RAR5 and RAR7 solid history | Two identical 1 MiB members, fixed-seed noise or repeated text | Explicit 1 MiB | 3 | 1 and 2 when parallel is enabled | 448 MiB |

The small matrix contains 84 writes with parallel enabled (42 sequential-only).
Its larger dictionary cases do not fill a 1 MiB history. The solid cases do:
two runs give the second run nonempty history and 2 MiB combined input, reaching
the promoted packed collector. Ordinary optimal parses exceed the 8,192-position
pricing-state ring. The larger ceiling accommodates conservative worker
reservations as well as live allocations; it is not a measured RSS target.

Successful cases require compression progress, byte-identical archives across
thread configurations, correct advertised dictionaries, integrity checks, and
byte-for-byte member decompression. Repeated refusal and cancellation cases
preserve an existing destination, release all managed capacity, leave no spool
files, and reuse the same resource group. A separate two-worker rendezvous
requires distinct workers to hold admitted capacity concurrently; its timeout
only prevents a scheduling failure from hanging the suite.

These are fixed ceilings on `WriterResources` managed allocation capacity and
reservations. Caller-owned input/output, untracked allocations, stacks, runtime,
allocator overhead and OS memory are outside that ledger. Progress observations
sample its usage; they are not an allocator high-water measurement. Existing
codec tests separately check ring storage dimensions and allocation-refusal
cleanup. None of these deterministic assertions claims a process-RAM bound or
a general memory improvement.

## Optional frozen promotion comparison

`compare.py` uses the unchanged `exact-encoder/vendor/worker.py` wait4 accounting
(SHA256 `866235a79a0b6e0bde6873efcb17613219a4736c5e74913c26b1611bd880771d`).
It requires standard-library Python 3.9+, Git, Cargo/Rust, a native reference
decoder (`--reference-decoder`, or `RAR_DECODER`), and macOS, Linux, or FreeBSD. No proprietary executable is included.

Preparation creates separate clean detached source checkouts, builds release
CLIs with `cargo build --release --locked -p rars-cli`, and freezes their source
commit/tree IDs, Cargo.lock hashes, executable SHA256s, compiler/Cargo versions,
relevant build environment, helper hashes, native decoder hash, and inputs.
The before control is `51addb73f98cc570e0d9989c8630cb71e835ae5f`; the promoted
default is PR #83 merge `4298f18e260ca18419053f49e9d7ca4d80e99fe0`. Both use
default features; **do not enable `experimental-ring-state` on the before
control**. The comparison measures the promotion, not a modified candidate.

Keep generated sources, binaries, archives, logs, and extraction directories
outside the repository. Preparation builds both binaries; measurement performs
no builds. Use a fresh output directory and a quiet host for measurement.
Use native Cargo builds with `CARGO_TARGET_DIR` and `CARGO_BUILD_TARGET` unset;
preparation expects each checkout's default `target/release/rars` location.

```sh
python3 benchmarks/encoder-memory/compare.py prepare \
  --repository /absolute/rars --reference-decoder /absolute/reference-decoder \
  --output /absolute/encoder-memory-run
python3 /absolute/encoder-memory-run/compare.py run \
  --output /absolute/encoder-memory-run
```

The fixed matrix contains two 1 MiB members per archive (2 MiB total), generated
either with SHAKE256 from the seeds `rars-memory-v1/0` and `rars-memory-v1/1`, or
by repeating the documented byte pattern in the script. File mode is 0644 and
mtime is 1700000000. Inputs and settings are identical for both controls:

| Requested format | Dictionary | Level | CLI threads | Members |
| --- | ---: | ---: | ---: | ---: |
| RAR5 (`rar50`) | 1 MiB | 1 | 1 | 2 |
| RAR7 (`rar70`) | 3 MiB | 3 | 2 | 2 |

Filters and solid mode are disabled. Estimated-workspace and managed-allocation
limits are both 1 GiB, allowing two jobs for the configured two-thread case.
The two independent members provide parallel work; the benchmark records the
configured thread count and does not instrument observed worker overlap.
This is a bounded sample, not a full dictionary/level/thread matrix or a
maximum-memory stress test.

Each workload/configuration gets one warmup pair and three measured pairs,
with deterministic alternating before/promoted order. All 32 archives must
pass the native reference decoder and their own encoder's integrity and extraction commands;
the extracted member set and SHA256s must match. Verification occurs outside
the measured encode process. Individual encode RSS, CPU and wall observations,
archive hashes, and all command logs are retained. Results never overwrite a
previous run. Timeouts and failures stop the run and retain evidence.

`wait4.ru_maxrss` is bytes on macOS and KiB converted to bytes on Linux/FreeBSD.
It includes process startup and all encoder threads, not extraction processes.
No cache eviction, baseline subtraction, confidence interval, statistical gate,
or broad performance claim is made. Three observations are descriptive only;
the script reports their raw RSS values and median. Different data, dictionary,
levels, threads or allocator behavior can produce different memory results.

## Local execution receipts

[observed-local.json](observed-local.json) records the frozen identities and all
observations from October 22, 2026 on arm64 macOS 26.6.2 with Homebrew Rust 1.97.1
and a native reference decoder. Two preliminary runs completed all 32 archive checks each,
but overlapped concurrent project compilation/tests because coordination
messages crossed. Both attempts are explicitly marked interfered and retained
in full. A third identical run followed explicit confirmation that the project
builds/tests and reviewer were idle. All 32 archives in that final run passed
both native and encoder integrity/extraction checks. Normal desktop background
processes remained; no CPU isolation or cache eviction was performed. Every
attempt uses identical frozen settings, sources, executables and inputs, with
no sample selection.

The final coordinated run observed these median process peaks, in MiB, across
three measured pairs after one warmup pair per workload/configuration:

| Data | Configuration | Before | Promoted |
| --- | --- | ---: | ---: |
| Incompressible | RAR5, 1 MiB dictionary, level 1, one thread | 18.031 | 16.422 |
| Incompressible | RAR7, 3 MiB dictionary, level 3, two threads | 40.547 | 33.344 |
| Repetitive | RAR5, 1 MiB dictionary, level 1, one thread | 10.438 | 9.969 |
| Repetitive | RAR7, 3 MiB dictionary, level 3, two threads | 89.547 | 43.406 |

These are local observations for this 2 MiB input protocol. Three samples do
not establish a statistical regression bound or a general memory improvement,
and do not resolve the different random-input result reported in PR #83.
All raw RSS/CPU/wall observations, including the two interfered attempts, remain
in the receipt. No unexecuted dictionary, level, thread, platform, or workload
is qualified by these measurements.
