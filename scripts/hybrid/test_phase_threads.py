"""Frozen phase thread counts, exercised with the existing fake archiver."""
import importlib.util
import os
from pathlib import Path
import sys
import time
import unittest


spec = importlib.util.spec_from_file_location(
    "hybrid_runner_test_fixture", Path(__file__).with_name("test_runner.py"))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
runner = fixture.runner


class PhaseThreadTests(unittest.TestCase):
    # Reuse setup helpers without inheriting the existing test methods.
    setUp = fixture.RunnerTests.setUp
    tearDown = fixture.RunnerTests.tearDown
    freeze = fixture.RunnerTests.freeze

    def test_execution_settings_are_normalized_and_invalid_settings_refused(self):
        defaults = {"parallel_screening": False, "screen_threads": 1, "confirm_threads": 1}
        self.assertEqual(runner.execution_settings(), defaults)
        self.assertEqual(runner.execution_settings({}), defaults)
        self.assertEqual(runner.execution_settings({"confirm_threads": 16}),
                         {**defaults, "confirm_threads": 16})
        for settings in ([], "threads", 1, True,
                         {"unknown": 1}, {"parallel_screening": 1},
                         {"parallel_screening": "true"},
                         {"screen_threads": True}, {"confirm_threads": False},
                         {"screen_threads": 0}, {"confirm_threads": -1},
                         {"screen_threads": 1.5}, {"confirm_threads": "16"}):
            with self.subTest(settings=settings), self.assertRaises(runner.RunError):
                runner.execution_settings(settings)

    def test_frozen_execution_and_all_encoder_phase_arguments(self):
        execution = {"parallel_screening": True, "screen_threads": 1, "confirm_threads": 16}
        manifest = self.freeze(execution=execution)
        self.assertEqual(manifest["execution"], execution)
        self.assertEqual(runner.phase_threads(manifest, "screen"), 1)
        self.assertEqual(runner.phase_threads(manifest, "confirm"), 16)
        self.assertEqual(runner.phase_threads({}, "confirm"), 1)
        case = manifest["cases"][0]
        configuration = runner.configs(["desktop"])[63]
        archive = self.root / "archive.rar"
        roles = ("native", "baseline", "candidate", "relationships", "streams", "native_ordered")
        for role in roles:
            commands = {}
            for phase, threads in (("screen", 1), ("confirm", 16)):
                command = runner.compression_command(
                    manifest, self.run, role, configuration, case, archive,
                    case["archive_members"] if role == "native_ordered" else None,
                    phase=phase)
                commands[phase] = command
                if role in ("native", "native_ordered"):
                    self.assertIn(f"-mt{threads}", command)
                    self.assertIn("-md128k", command)
                else:
                    self.assertEqual(command[command.index("--threads") + 1], str(threads))
                    for option in ("--memory-limit", "--max-memory"):
                        self.assertEqual(command[command.index(option) + 1], str(256 * 1024**2))
                    if role == "baseline":
                        self.assertEqual(command[command.index("--dict-size") + 1], str(128 * 1024))
                    else:
                        self.assertNotIn("--dict-size", command)
                        self.assertIn("--auto-filter", command)
                        self.assertEqual("--analyze-relationships" in command, role != "streams")
                        self.assertEqual("--optimize-member-order" in command, role != "streams")
            # Thread count is the only command change between phases.
            if role in ("native", "native_ordered"):
                self.assertEqual(["-mt1" if arg == "-mt16" else arg for arg in commands["confirm"]],
                                 commands["screen"])
            else:
                normalized = commands["confirm"][:]
                normalized[normalized.index("--threads") + 1] = "1"
                self.assertEqual(normalized, commands["screen"])

    def test_host_reservations_refuse_overcommit_and_weighted_assignment_is_stable(self):
        self.inventory = self.root / "inventory.json"
        host = {"id": "first", "kind": "export", "hostname": "first-host",
                "screening_slots": 2, "logical_cpus": 4,
                "memory_budget_bytes": 512 * 1024**2}
        execution = {"parallel_screening": True, "screen_threads": 2, "confirm_threads": 4}
        for override, settings, error in (
                ({"screening_slots": 3}, execution, "CPU allowance"),
                ({}, {**execution, "confirm_threads": 5}, "CPU allowance"),
                ({"memory_budget_bytes": 512 * 1024**2 - 1}, execution, "managed-memory allowance")):
            runner.write_json(self.inventory, [{**host, **override}])
            with self.subTest(host=override, execution=settings), self.assertRaisesRegex(runner.RunError, error):
                self.freeze(execution=settings)
            self.assertFalse(self.run.exists())

        hosts = [{**host, "screening_slots": 1, "memory_budget_bytes": 256 * 1024**2},
                 {**host, "id": "second", "hostname": "second-host", "screening_slots": 3,
                  "memory_budget_bytes": 768 * 1024**2}]
        runner.write_json(self.inventory, hosts)
        manifest = self.freeze(execution={**execution, "screen_threads": 1})
        assignments = set()
        for configuration in manifest["configurations"]:
            # An independently calculated 1:3 slot allocation catches accidental
            # equal-host assignment, while varied cases/phases catch migration.
            expected = "first" if int(runner.digest(configuration["id"]), 16) % 4 == 0 else "second"
            for phase in ("screen", "confirm"):
                for workload in ("small", "different-workload"):
                    for repetition in (-1, 0, 14):
                        task = {"phase": phase, "configuration": configuration["id"],
                                "case": workload, "repetition": repetition}
                        self.assertEqual(runner.host_for(manifest, task), expected)
            assignments.add(expected)
        self.assertEqual(assignments, {"first", "second"})

    def test_phase_threads_reach_integrity_extraction_and_cold_selective(self):
        helper = self.root / "cache-prepare"
        helper.write_text(f"#!{sys.executable}\n# Test helper: no cache eviction is claimed.\n")
        helper.chmod(0o755)
        manifest = self.freeze(
            execution={"parallel_screening": True, "screen_threads": 1, "confirm_threads": 16},
            cold_prepare=[str(helper)])
        case = manifest["cases"][0]
        configuration = manifest["configurations"][0]
        for phase, threads in (("screen", 1), ("confirm", 16)):
            area = self.root / phase
            area.mkdir()
            task = {"phase": phase, "configuration": configuration["id"],
                    "case": case["id"], "repetition": 0}
            result = {"roles": {}}
            runner.run_role(self.run, manifest, task, configuration, case, area,
                            time.monotonic() + 30, {**os.environ, "LC_ALL": "C", "TZ": "UTC"},
                            result, "candidate")
            role = result["roles"]["candidate"]
            self.assertEqual(role["encode"]["command"][2], str(threads))
            for decoder in ("native", "candidate"):
                for operation in ("integrity", "extract"):
                    record = runner.read_json(area / f"candidate-{decoder}-{operation}.json")
                    command = record["command"]
                    if decoder == "native":
                        self.assertIn(f"-mt{threads}", command)
                    else:
                        self.assertEqual(command[command.index("--threads") + 1], str(threads))
            cold = role["cold_selective"]
            self.assertTrue(cold["available"])
            self.assertIn(f"-mt{threads}", cold["extract"]["command"])
            self.assertEqual(cold["requested_member"], "folder/data.txt")


if __name__ == "__main__":
    unittest.main()
