"""SSH retries retain computation state but refresh transport snapshots."""
import shutil
import socket
import subprocess
import unittest
from unittest import mock
import test_runner as fixtures

runner = fixtures.runner


class SshRecoveryTests(unittest.TestCase):
    setUp = fixtures.RunnerTests.setUp
    tearDown = fixtures.RunnerTests.tearDown
    freeze = fixtures.RunnerTests.freeze

    def test_retry_keeps_remote_state_and_reexports_results(self):
        self.inventory = self.root / "hosts.json"
        host = {"id": "remote", "kind": "ssh", "hostname": socket.gethostname(),
                "ssh_target": "example.invalid", "remote_directory": "/work/runner"}
        runner.write_json(self.inventory, [host])
        self.freeze()
        source = self.root / "remote-state"
        shutil.copytree(self.run, source)
        returned = self.root / "snapshot.tar.gz"
        runner.export_worker(source, "remote", returned)
        commands = []
        runs = []
        def transport(command):
            commands.append(command)
            if command[0] == "scp" and command[-1].endswith("results.tar.gz"):
                shutil.copy2(returned, command[-1])
            return ""
        def execute(command, **kwargs):
            runs.append(command[-1])
            return subprocess.CompletedProcess(command, 0, stdout="", stderr="")
        with mock.patch.object(runner, "checked_command", side_effect=transport), \
             mock.patch.object(runner.subprocess, "run", side_effect=execute):
            runner.dispatch_ssh(self.run, host)
            runner.append(self.run, {"event": "phase-done", "phase": "screen", "host": "remote"})
            runner.dispatch_ssh(self.run, host)
        self.assertEqual(runs[0], runs[1], "journal progress must not change the worker state directory")
        exports = [c[-1] for c in commands if c[0] == "ssh" and " export-worker " in c[-1]]
        self.assertEqual(len(exports), 2)
        self.assertNotEqual(exports[0], exports[1], "each attempt needs a fresh result bundle")
        self.assertTrue(all("test -f" not in command for command in exports))
        self.assertEqual(len(list((self.run / "transport").glob("*/*/*/returned"))), 0)
        self.assertEqual(len(list((self.run / "transport").glob("*/*/*/transport.json"))), 2)

    def test_import_supplies_confirmation_selection_to_existing_worker(self):
        self.freeze()
        other = self.root / "worker"
        shutil.copytree(self.run, other)
        selection = {"manifest_sha256": runner.sha(self.run / "manifest.json"), "finalists": []}
        runner.write_json(self.run / "selection.json", selection)
        runner.import_results(other, self.run)
        self.assertEqual(runner.read_json(other / "selection.json"), selection)
        runner.write_json(self.run / "selection.json", {**selection, "finalists": ["desktop-00"]})
        with self.assertRaisesRegex(runner.RunError, "conflicting frozen selection"):
            runner.import_results(other, self.run)

    def test_identical_new_experiments_have_distinct_run_identity(self):
        first = self.freeze()
        self.run = self.root / "second-run"
        second = self.freeze()
        self.assertNotEqual(first["run_id"], second["run_id"])
        self.assertEqual(runner.tasks(first, "screen"), runner.tasks(second, "screen"))
