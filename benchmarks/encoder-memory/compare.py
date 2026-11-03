#!/usr/bin/env python3
"""Small serial RSS observation of the default encoder before/after PR #83.

Uses the existing exact-encoder worker's wait4 accounting. RSS is observational;
only successful encoding, extraction and stable frozen identities are asserted.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import time

REVISIONS = {
    "before": "51addb73f98cc570e0d9989c8630cb71e835ae5f",
    "promoted": "4298f18e260ca18419053f49e9d7ca4d80e99fe0",
}
WORKER_SHA = "866235a79a0b6e0bde6873efcb17613219a4736c5e74913c26b1611bd880771d"
CASES = [
    {"id": "rar5-1m-level1-thread1", "format": "rar50", "dictionary": 1048576, "level": 1, "threads": 1},
    {"id": "rar7-3m-level3-thread2", "format": "rar70", "dictionary": 3145728, "level": 3, "threads": 2},
]
MEMBERS = ("first.bin", "second.bin")
FIXED_TIME = 1700000000


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def checked(command, cwd=None, log=None):
    command = list(map(str, command))
    if log is None:
        return subprocess.check_output(command, cwd=cwd, text=True).strip()
    with log.open("x") as stream:
        subprocess.run(command, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT, check=True)


def prepare(arguments):
    require(sys.platform == "darwin" or sys.platform.startswith(("linux", "freebsd")), "Unsupported wait4 RSS units")
    require(arguments.reference_decoder is not None, "Pass --reference-decoder or set RAR_DECODER")
    output = arguments.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve()
    worker = source.parent.parent / "exact-encoder/vendor/worker.py"
    require(sha(worker) == WORKER_SHA, "Frozen accounting worker changed")
    shutil.copy2(source, output / "compare.py")
    shutil.copy2(worker, output / "worker.py")
    native = output / "reference-decoder"
    shutil.copy2(arguments.reference_decoder.resolve(strict=True), native)
    metadata = {
        "status": "preparing", "scope": "Local serial observations; no RSS threshold or general memory claim",
        "compiler": checked(["rustc", "--version", "--verbose"]),
        "cargo": checked(["cargo", "--version"]),
        "platform": platform.platform(), "machine": platform.machine(),
        "build_environment": {key: value for key, value in os.environ.items()
                              if key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTUP_TOOLCHAIN", "CARGO_BUILD_TARGET")
                              or key.startswith("CARGO_PROFILE_RELEASE_")},
        "cases": CASES, "repetitions": 3, "warmup_pairs": 1,
        "compare_sha256": sha(output / "compare.py"), "worker_sha256": sha(output / "worker.py"),
        "reference_decoder_sha256": sha(native), "sources": {}, "workloads": {},
        "memory_limit_bytes": 1073741824, "max_memory_bytes": 1073741824,
        "filter": "none", "solid": False,
    }
    write(output / "manifest.json", metadata)
    for role, revision in REVISIONS.items():
        tree = output / ("source-" + role)
        checked(["git", "clone", "--shared", "--no-checkout", arguments.repository.resolve(), tree], log=output / (role + "-clone.log"))
        checked(["git", "checkout", "--detach", revision], cwd=tree, log=output / (role + "-checkout.log"))
        require(checked(["git", "status", "--porcelain", "--untracked-files=all"], cwd=tree) == "", "Unclean source clone")
        command = ["cargo", "build", "--release", "--locked", "-p", "rars-cli"]
        # In particular: never enable experimental-ring-state in the before control.
        checked(command, cwd=tree, log=output / (role + "-build.log"))
        binary = output / role
        shutil.copy2(tree / "target/release/rars", binary)
        metadata["sources"][role] = {
            "revision": checked(["git", "rev-parse", "HEAD"], cwd=tree),
            "tree": checked(["git", "rev-parse", "HEAD^{tree}"], cwd=tree),
            "cargo_lock_sha256": sha(tree / "Cargo.lock"),
            "command": command, "features": "default only", "binary_sha256": sha(binary),
        }
        write(output / "manifest.json", metadata)
    for workload in ("incompressible", "repetitive"):
        directory = output / "inputs" / workload
        directory.mkdir(parents=True)
        records = {}
        for index, name in enumerate(MEMBERS):
            if workload == "incompressible":
                payload = hashlib.shake_256(("rars-memory-v1/" + str(index)).encode()).digest(1048576)
            else:
                pattern = b"RAR default encoder memory coverage\x00\xff" + bytes([index])
                payload = (pattern * (1048576 // len(pattern) + 1))[:1048576]
            path = directory / name
            path.write_bytes(payload)
            path.chmod(0o644)
            os.utime(path, (FIXED_TIME, FIXED_TIME))
            records[name] = {"bytes": len(payload), "sha256": sha(path)}
        metadata["workloads"][workload] = records
    metadata["status"] = "prepared"
    write(output / "manifest.json", metadata)
    (output / "manifest.sha256").write_text(sha(output / "manifest.json") + "\n")
    print("Prepared frozen default binaries and inputs at " + str(output))


def run(output):
    require(sha(output / "manifest.json") == (output / "manifest.sha256").read_text().strip(), "Manifest changed")
    metadata = json.loads((output / "manifest.json").read_text())
    require(sha(Path(__file__)) == metadata["compare_sha256"], "Run the frozen compare.py")
    require(sha(output / "worker.py") == metadata["worker_sha256"] == WORKER_SHA, "Accounting worker changed")
    require(sha(output / "reference-decoder") == metadata["reference_decoder_sha256"], "Native decoder changed")
    for role, source in metadata["sources"].items():
        require(sha(output / role) == source["binary_sha256"], "Encoder changed: " + role)
    for workload, members in metadata["workloads"].items():
        for name, record in members.items():
            path = output / "inputs" / workload / name
            require(path.stat().st_size == record["bytes"] and sha(path) == record["sha256"], "Input changed")
    samples = output / "samples"
    samples.mkdir(exist_ok=False)  # Never overwrite or silently retry previous observations.
    spec = importlib.util.spec_from_file_location("rss_worker", output / "worker.py")
    worker = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(worker)
    environment = {key: value for key, value in os.environ.items() if not key.startswith("RARS_")}
    environment.update({"LC_ALL": "C", "TZ": "UTC"})
    result = {"status": "running", "manifest_sha256": sha(output / "manifest.json"), "samples": []}
    write(output / "results.json", result)
    try:
        for workload in metadata["workloads"]:
            for case in metadata["cases"]:
                for repetition in range(-1, metadata["repetitions"]):
                    # Alternate the pair order deterministically; retain all samples.
                    roles = ("before", "promoted") if repetition % 2 else ("promoted", "before")
                    for role in roles:
                        area = samples / (workload + "-" + case["id"] + "-" + str(repetition) + "-" + role)
                        area.mkdir()
                        archive = area / "archive.rar"
                        binary = output / role
                        env = {**environment, "RAYON_NUM_THREADS": str(case["threads"])}
                        command = [binary, "--threads", str(case["threads"]), "--progress", "never", "a",
                                   "--format", case["format"], "--level", str(case["level"]),
                                   "--dict-size", str(case["dictionary"]), "--memory-limit", str(metadata["memory_limit_bytes"]),
                                   "--max-memory", str(metadata["max_memory_bytes"]), "--no-filter", archive, *MEMBERS]
                        measured = worker.execute(list(map(str, command)), output / "inputs" / workload,
                                                  area, "encode", time.monotonic() + 120, env)
                        archive_hash = sha(archive)
                        for decoder in ("native", "self"):
                            target = area / decoder
                            target.mkdir()
                            if decoder == "native":
                                test = [output / "reference-decoder", "t", "-idq", "-p-", "-mt1", archive]
                                extract = [output / "reference-decoder", "x", "-idq", "-y", "-p-", "-mt1", archive, str(target) + "/"]
                            else:
                                test = [binary, "--threads", "1", "test", archive]
                                extract = [binary, "--threads", "1", "extract", archive, target]
                            for label, invocation in (("test", test), ("extract", extract)):
                                worker.execute(list(map(str, invocation)), area, area, decoder + "-" + label,
                                               time.monotonic() + 120, env)
                            require(sorted(path.name for path in target.iterdir()) == sorted(MEMBERS), "Unexpected extracted members")
                            for name, record in metadata["workloads"][workload].items():
                                require((target / name).is_file() and sha(target / name) == record["sha256"], "Extraction differs")
                        require(sha(archive) == archive_hash, "Archive changed during verification")
                        result["samples"].append({
                            "workload": workload, "case": case["id"], "repetition": repetition, "role": role,
                            "peak_rss_bytes": measured["peak_rss_bytes"], "wall_seconds": measured["wall_seconds"],
                            "cpu_seconds": measured["cpu_seconds"], "archive_sha256": archive_hash,
                            "archive_bytes": archive.stat().st_size, "native_and_self_test_extraction": "pass",
                        })
                        write(output / "results.json", result)
        for role, source in metadata["sources"].items():
            require(sha(output / role) == source["binary_sha256"], "Encoder changed during run")
        require(sha(output / "reference-decoder") == metadata["reference_decoder_sha256"], "Decoder changed during run")
        result["summary"] = []
        for workload in metadata["workloads"]:
            for case in metadata["cases"]:
                summary = {"workload": workload, "case": case["id"]}
                for role in metadata["sources"]:
                    rows = [row for row in result["samples"] if row["workload"] == workload and row["case"] == case["id"]
                            and row["role"] == role and row["repetition"] >= 0]
                    summary[role] = {metric: statistics.median(row[metric] for row in rows)
                                     for metric in ("peak_rss_bytes", "cpu_seconds", "wall_seconds")}
                    summary[role]["rss_samples_bytes"] = [row["peak_rss_bytes"] for row in rows]
                result["summary"].append(summary)
        result["status"] = "pass"
    except BaseException as error:
        result["status"] = "failed"
        result["error"] = str(error)
        raise
    finally:
        write(output / "results.json", result)
    print("All 32 archives passed native and encoder integrity/extraction checks; RSS observations: " + str(output / "results.json"))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare_parser = commands.add_parser("prepare")
    prepare_parser.add_argument("--repository", type=Path, required=True)
    prepare_parser.add_argument("--reference-decoder", type=Path, default=os.environ.get("RAR_DECODER") or None)
    prepare_parser.add_argument("--output", type=Path, required=True)
    run_parser = commands.add_parser("run")
    run_parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    if arguments.command == "prepare":
        prepare(arguments)
    else:
        run(arguments.output.resolve())
