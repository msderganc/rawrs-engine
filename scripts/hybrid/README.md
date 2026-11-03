# Frozen encoder comparison runner

This POSIX Python 3.9+ runner executes a finite experiment without building or
changing source, calling an AI service, or discovering remote hosts. It requires
an already-built ordinary `rars`; comparison experiments also require a
native RAR executable. The default compares those two encoders, with one configuration per resource
profile. It runs against the current CLI without any hybrid encoder changes.
An optional `--baseline` adds a separate ordinary-rars executable, for example
a previous release. The 64-way hybrid matrix remains available only through
`--experiment hybrid` with an external compatible experimental executable.
It reports measurements and verified Pareto candidates;
it does not assert native parity or apply a size-percentage or archive-byte
identity acceptance rule.

`--experiment smoke` runs only ordinary rars creation, integrity testing and
extraction through the same frozen worker and receipts. It requires no native
RAR executable and does not select finalists, run confirmation, rank encoders or
establish cross-decoder interoperability. Native, baseline and cold-extraction
controls are refused in this mode.

Local and SSH workers support macOS, Linux and FreeBSD with compatible frozen
executables. Peak RSS uses macOS byte units and converts Linux/FreeBSD KiB units
to bytes; unknown platforms retain an unavailable RSS measurement.

Generate the related-release, distance, shifted and unrelated fixtures (omit
`--base-corpus` to use just these small synthetic inputs):

```sh
python3 scripts/hybrid_fixtures.py work/hybrid-corpus \
  --base-corpus /absolute/path/to/existing/manifest.json
```

Use an ignored working directory or a location outside the source checkout for generated data. From a **clean committed checkout**, freeze the binaries, full corpus and plan:

```sh
python3 scripts/hybrid_candidates.py init --output work/hybrid-run \
  --rars /absolute/path/to/rars \
  --native-rar /absolute/path/to/native-rar \
  --corpus work/hybrid-corpus/manifest.json \
  --profile desktop --profile server
python3 work/hybrid-run/frozen/hybrid_candidates.py run work/hybrid-run
python3 work/hybrid-run/frozen/hybrid_candidates.py status work/hybrid-run
```

Initialization checks the supplied rars add-command help for the required CLI
options before staging a workload. Hybrid mode additionally requires its
experimental switches and refuses an ordinary binary before creating a run.
This is an interface check; extraction verification during execution establishes
correctness for the tested inputs.

Use `--experiment hybrid --rars /path/to/experimental-rars` to opt into the
archived matrix protocol. This repository does not supply that encoder.
In default controls mode, both rars and native RAR use the same fitted dictionary
and matched thread count. A controls-only plan may instead freeze
`fixed_dictionary_bytes` for every workload and encoder role. No relationship/stream controls or ordering-assisted
native pass are requested. The screening/confirmation workflow, frozen evidence
and paired measurements below apply to either mode.

Profiles may be selected independently; the default is desktop. Do not modify binaries, frozen inputs, manifests or frozen
worker source after initialization. A different experiment needs a new directory.
`run --max-tasks N` pauses at a task boundary; the same `run` resumes automatically.

In optional hybrid mode, each profile has exactly 64 immutable variants. Bit 0 enables deeper search,
bit 1 multiple states, bit 2 full partitions, bit 3 automatic filters, bit 4
member ordering, and bit 5 adaptive rather than existing automatic dictionary
selection. Every combined-hybrid variant explicitly requests relationship
analysis; bit 4 changes ordering alone. Hybrid, relationships and streams omit
an explicit dictionary request. The filter bit explicitly requests `--auto-filter` when on and `--no-filter` when off; a separately requested mandatory filter remains authoritative. Solid/filter combinations unsupported by the CLI are frozen as pre-dispatch exclusions with no child launch; all 64 configurations remain represented. Eligibility and confirmation require every applicable valid workload, at least one valid workload, and report omitted coverage explicitly. Configurations covering different applicable subsets are not evidence of a universal winner.

All encoders use level 3. The default thread count is one; optional frozen
execution settings can request separate matched screening/confirmation thread
counts for every encoder and decoder. Ordinary rars/native controls request
the same mechanically fitted dictionary: the next power of two reaching the
largest regular member (the sum for solid workloads), at least 128 KiB, clipped
to 4 MiB for desktop or 64 MiB for server. These settings are recorded per
workload/profile in the frozen manifest and actual command. All rars encoder
roles explicitly set both estimated-workspace and hard managed-memory limits
to 256 MiB desktop or 8 GiB server. Worker options cannot override the matrix's
dictionary, thread or memory switches. Native process RSS is measured independently.
Rars requests `rar50`; native RAR requests `-ma5`.
Actual native archive metadata is retained verbatim: these flags do not establish
an identical compression-algorithm version. Corpus workloads control solid mode.

In hybrid mode, every paired sample also runs relationship-only and stream-only architecture
controls. Relationships explicitly requests whole-archive analysis and permits
ordering only when bit 4 is set; streams masks bits 4/5, preserves the original
order and uses existing automatic dictionary selection, enabling only the stream-encoder choices. The
five unassisted roles (ordinary native, ordinary rars, hybrid, relationships,
streams) have seeded random order. A sixth **ordering-assisted native** role
runs afterward with `-ds`, using the candidate archive's member sequence. Its
actual member sequence must also match; it is reported separately from ordinary
native. Bare listings supply ordering only, after the candidate archive has
already passed actual two-decoder extraction verification. Corpus names with
newlines are refused rather than ambiguously parsed.

The corpus schema is `{"files":[{"path":"relative/file","sha256":"..."}],
"workloads":[{"id":"case","files":["relative/file"],"solid":false}]}`.
The existing parity corpus manifest is supported directly. Explicit empty
directories may appear as `{"path":"empty","type":"directory"}`. Each
workload gets an isolated frozen tree with exactly its files and parent/empty
directories. Only explicitly requested paths are passed to archivers. Implicit
parents are checked for existence/path/type, without inventing a request to
preserve their modes or timestamps. Explicit directories retain strict metadata
checks, including empty ones; a native extractor that changes a requested
folder timestamp while creating children fails that check and is recorded as a
control limitation.

Screening has a maximum **four aggregate host-hours**, confirmation twelve.
Multiple hosts receive fixed equal shares, so disconnected/exported workers
cannot multiply the total allowance. Each task defaults to a 300-second limit;
the frozen plan may override it. This includes encoding, decoding and correctness
work. The process group is killed
when its task reservation expires; normal scheduler/termination overhead is
recorded, not hidden. Task deadlines precede validation and are checked while hashing and verifying
extractions. Active phase wall time includes scheduling, journals, selection and
correctness work. OS I/O blocking and process-termination latency cannot be
preempted, so this is a cooperative budget, not a hard real-time guarantee.
Interrupted tasks conservatively consume their full reservation. Failed or interrupted samples are not automatically retried. Phase-budget exhaustion and cancellation are recorded as interrupted; they do not imply corrupt inputs or a failed comparison control. Per-task timeouts remain failures and exclude the configuration under the configured failure policy. Incomplete configurations remain ineligible for promotion.
Control failure stops the run; candidate failure excludes that configuration by
default. Set `execution.screen_failure_policy` to `"workload"` to retain a failed
screening task and continue that configuration's other workloads. Such partial
coverage remains diagnostic and cannot enter finalist confirmation; confirmation
failures still exclude the configuration.
A screening timeout is a screening refusal, not proof of incorrect extraction.

### CPU-parallel screening and exclusive confirmation

Set the optional plan execution settings before freezing:

```json
{"execution":{"parallel_screening":true,"screen_threads":1,"confirm_threads":16}}
```

Each inventory entry supplies `screening_slots`, with optional `logical_cpus`
and `memory_budget_bytes` admission bounds. For example, 32 slots on a 40-CPU
host and 80 on a 96-CPU host leave capacity for the OS. The maximum controlled
memory per screening job is reserved in the frozen admission calculation;
explicit CPU/memory bounds reject an overcommitted plan before staging.
Configurations are assigned deterministically, weighted by those slot counts.
Each slot processes one configuration's workloads in order. Completed receipts
commit in preplanned configuration/workload order regardless of completion order.

The host-wide lock permits one coordinator. Screening may run its reserved
slots concurrently, with one shared host phase deadline. Host-hours count elapsed
host time once, not the sum of simultaneous job durations. Durable phase
reservations charge an interrupted/crashed screening phase once, conservatively.
Individual job timing and CPU use remain in the receipts. Screening timings are
affected by CPU, memory and I/O contention and are diagnostic ranking inputs.

All screening jobs drain on both hosts before finalist selection. Confirmation
uses one job per host with the frozen matched thread counts for rars/native RAR
encoding, integrity tests and extraction. It never overlaps its own screening
or another confirmation on that host. Only complete isolated confirmation can
qualify a finalist; requesting threads does not imply every encoder stage can
use them. Resume retains verified completions and the frozen execution settings.

Screening visits seeded whole-configuration workload blocks, keeping each configuration on one explicitly assigned host so a budget cutoff can yield complete candidates. It uses one sample for every configuration/workload it reaches. Only configurations
with all applicable workloads verified can enter selection. Four-dimensional non-dominance
uses within-task candidate/native ratios for compression wall time, archive size, compression peak RSS and rars extraction wall time, with equal workload weights. Absolute per-workload/host frontiers remain in the report; unlike hosts are not ranked by raw seconds or RSS. Up to three frontier configurations per
profile advance using a deterministic sum-log compromise rank. Configurations
with different applicable workloads are ranked in separate coverage groups;
promotion visits those groups round-robin, with broader coverage first. Confirmation
uses one warm-up followed by exactly 15 paired repetitions per workload, with
all unassisted command order frozen by task seed. Incomplete
confirmation is reported as incomplete, not qualified. Raw per-process results
and paired control ratios remain available for independent analysis.

Every archive is integrity-tested and **actually extracted by both native RAR
and rars**. Verification checks every file/directory path (including empty dirs),
file size and SHA256, POSIX permission bits and whole-second modification times of explicitly requested
archive members; implicit parent directories require path/type only;
symlinks are never followed. Owner/group, ACLs, xattrs and subsecond timestamps
are outside the supported comparison. No listing-only verification is used.
Wall time, child CPU and per-process peak RSS are recorded. Kernel filesystem
block counters and materialized temporary-output bytes have explicit scopes;
I/O byte totals and cold-cache claims are unavailable by default. Successful extracted trees are removed only after durable, hash-bound member and verification receipts are saved. Archives, receipts and logs remain; failed extraction trees are preserved. Provide disk space for the retained archives. A nonzero exit, content/metadata mismatch, resource refusal or timeout
retains its diagnostic files and excludes the affected candidate.

Optional `--plan plan.json` accepts the execution settings above, `worker_options` arrays (`rars_add`,
`native_add`, `rars_read`, `native_read`), a positive
`task_timeout_seconds`, and reduced `budgets.screen`/`budgets.confirm` in seconds.
For controls experiments, `fixed_dictionary_bytes` selects one power-of-two
dictionary of at least 128 KiB, at most 4 MiB when desktop is selected or 64 MiB
for server alone. Initialization rejects invalid values before staging. The
value and each workload's resulting resources are frozen and checked on resume;
candidate rars, baseline rars and native RAR receive the same dictionary.
Hybrid experiments reject this override so their adaptive-dictionary comparison
remains unchanged. For example, a fixed-parser/filter cache comparison can use:

```json
{
  "fixed_dictionary_bytes": 4194304,
  "worker_options": {"rars_add": ["--no-filter"], "native_add": ["-mc-"]},
  "execution": {"parallel_screening": false, "screen_threads": 1, "confirm_threads": 1}
}
```

Controls mode accepts explicit `--no-filter` or `--auto-filter` in `rars_add`
and passes the same options to candidate and baseline; native filter settings
must be supplied independently as above. The runner fixes level 3 for all
encoders. Native RAR uses its own parser, while both rars binaries receive the
same parser settings.
Budgets cannot exceed the declared maxima. The six matrix switches cannot be
injected through worker options in hybrid mode.

For optional cold selective extraction, supply a direct executable helper in the
same plan:

```json
{"cold_prepare":["/absolute/path/cache-helper","{archive}"]}
```

Initialization copies and hashes this executable; workers run only that frozen
copy with literal argument arrays, without a shell. An argument equal to
`{archive}` becomes the measured archive's absolute path. Other arguments remain
literal. Shell/privilege wrappers are refused; no purge or sudo is implicit.
For each role, after the mandatory full extraction by both decoders, the runner
invokes this helper and measures native extraction of the lexicographically last
requested regular file. It verifies that selected file's bytes, path, supported
metadata and implicit parent paths, and checks the archive stayed unchanged.
Helper and extraction commands, wall time, CPU and RSS are recorded within the
same task budget. Successful selective trees are removed after durable proof;
failed trees remain. The scope is **operator-supplied cache preparation**:
system/global cache eviction is not verified. Without this option, or for a
workload containing no requested regular file, the measurement is unavailable.

## Explicit workers

No inventory means this host only; the runner does not search for SSH hosts.
An optional `--inventory hosts.json` freezes an explicit array:

```json
[
  {"id":"local","kind":"local","hostname":"desktop.example"},
  {"id":"worker1","kind":"ssh","hostname":"worker1.example",
   "ssh_target":"user@worker1.example","port":22,
   "remote_directory":"/work/rars-hybrid"}
]
```

The declared `hostname` must equal Python's `socket.gethostname()` on that
worker. Supply compatible frozen executable architectures. SSH uses BatchMode
and a connection timeout; it does not change SSH trust/credentials or accept
unknown keys automatically. Run `dispatch work/hybrid-run` to execute only the
listed local/SSH workers, import their hash-bound receipts and advance the two
phases mechanically. Hosts dispatch concurrently within each phase; returned
receipts retain inventory order and selection waits for the whole phase.
Local subprocesses use argument arrays; remote paths and
commands are validated/quoted. No actual SSH is exercised by the unit tests.

For an offline worker, declare `kind:"export"` and use:

```sh
python3 scripts/hybrid_candidates.py export-worker work/hybrid-run \
  --host worker1 --output worker1.tar.gz
# On the explicitly selected worker: extract the generated bundle, then
python3 hybrid-run/frozen/hybrid_candidates.py run hybrid-run --host worker1
# Return that directory (or a newly exported result bundle), then on controller:
python3 scripts/hybrid_candidates.py import-results work/hybrid-run \
  --from-directory /absolute/path/to/returned/hybrid-run
```

Only metadata/command receipts need returning alongside the frozen manifest and
inputs; exported bundles retain these. Do not merge an active worker's directory.
The controller waits for every worker's screen-phase boundary before selecting
finalists. Re-export after selection to supply the immutable confirmation list.
Transport failures retain a stable remote state directory for each frozen run. Each retry transfers fresh snapshots, merges monotonic journal progress and preserves outstanding budget reservations; task IDs and receipts prevent silently
rerunning completed samples. Conflicting receipts, changed hashes and torn
journal lines fail closed and preserve evidence for repair.

Run the mechanical tests, including fake executable smoke, with:

```sh
python3 -m unittest discover -s scripts/hybrid -v
```

The unit tests use fake archive executables to prove scheduling, command and
verification behavior. They do not establish native RAR interoperability or
performance. Real-binary integration tests skip explicitly when their executable
dependencies are unavailable. To run them from a **clean committed checkout**:

```sh
cargo build -p rars-cli --locked
RARS_SMOKE_BINARY="$PWD/target/debug/rars" \
  python3 -m unittest discover -s scripts/hybrid -v
```

The bounded rars integration test uses two tiny workloads and pauses after one
task, resumes the frozen worker, and checks that the first completed receipt and
artifacts are unchanged. It checks real RAR archive bytes, integrity testing,
extraction, file content, POSIX permissions, whole-second modification times and
an explicit empty directory. It retains the runner's executable/source hashes,
deterministic task IDs and receipts, one-thread desktop resource settings, and
short frozen task and aggregate budgets. This is correctness smoke coverage;
it does not run a performance campaign or the 15-repetition confirmation phase.

For the complete two-encoder smoke and both decoders on each encoder's archive,
supply a native RAR executable with `RARS_NATIVE_RAR=/absolute/path/to/native-rar`
alongside `RARS_SMOKE_BINARY`. Without it, a missing native encoder is an
explicit skip. A decoder is not an encoder substitute, and simulated ZIP
fixtures never count as native execution. The Linux CI job builds rars and runs
the suite with `RARS_SMOKE_BINARY`; native coverage depends on an available
native binary.

For an operator-supplied corpus, initialize the same self-roundtrip mode, then
repeat the frozen-worker command to resume at the next task:

```sh
python3 scripts/hybrid_candidates.py init --experiment smoke \
  --rars /absolute/path/to/rars --corpus /path/to/manifest.json \
  --output /path/to/run
python3 /path/to/run/frozen/hybrid_candidates.py run /path/to/run --max-tasks 1
```

A plan can reduce `task_timeout_seconds` and `budgets.screen` as documented
above. No campaign starts at installation time.
