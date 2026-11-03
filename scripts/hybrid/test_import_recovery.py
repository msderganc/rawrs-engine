"""Receipt transport must preserve progress across interrupted imports."""
import shutil
import unittest
from unittest import mock
import test_runner as fixtures

runner = fixtures.runner


class ImportRecoveryTests(unittest.TestCase):
    setUp = fixtures.RunnerTests.setUp
    tearDown = fixtures.RunnerTests.tearDown
    freeze = fixtures.RunnerTests.freeze
    def test_stale_started_receipt_does_not_replace_completion(self):
        self.experiment = "controls"
        self.freeze()
        other = self.root / "exported"
        original = runner.sample
        def snapshot(run, manifest, task, deadline):
            shutil.copytree(run, other)
            return original(run, manifest, task, deadline)
        with mock.patch.object(runner, "sample", side_effect=snapshot):
            runner.worker(self.run, "local", 1)
        before = runner.latest(runner.journal(self.run))
        self.assertEqual(runner.import_results(self.run, other)["imported_tasks"], 0)
        self.assertEqual(runner.latest(runner.journal(self.run)), before)

    def test_crashed_parallel_reservation_survives_import(self):
        m = self.freeze(execution={"parallel_screening": True})
        other = self.root / "exported"
        shutil.copytree(self.run, other)
        task = runner.tasks(m, "screen")[0]
        runner.append(other, {"event": "phase-reservation", "id": "crashed", "phase": "screen",
                              "host": "local", "reserved_seconds": 100})
        runner.append(other, {"event": "started", "task": task, "task_id": runner.task_id(task),
                              "host": "local", "phase_reservation": "crashed", "reserved_seconds": 30})
        runner.import_results(self.run, other)
        runner.import_results(self.run, other)
        self.assertEqual(runner.consumed(runner.journal(self.run), "screen", "local"), 100)

    def test_interrupted_copy_and_append_are_retryable(self):
        self.experiment = "controls"
        self.freeze()
        other = self.root / "exported"
        shutil.copytree(self.run, other)
        runner.worker(other, "local", 1)
        with mock.patch.object(runner.shutil, "copy2", side_effect=OSError("copy interrupted")):
            with self.assertRaises(OSError):
                runner.import_results(self.run, other)
        original = runner.append
        def fail_terminal(run, row):
            if row.get("event") == "complete":
                raise OSError("append interrupted")
            original(run, row)
        with mock.patch.object(runner, "append", side_effect=fail_terminal):
            with self.assertRaises(OSError):
                runner.import_results(self.run, other)
        self.assertEqual(runner.import_results(self.run, other)["imported_tasks"], 1)
        self.assertEqual(runner.import_results(self.run, other)["imported_tasks"], 0)
        self.assertEqual(len(runner.verified_results(self.run, runner.journal(self.run), "screen")), 1)


    def test_inflight_artifacts_do_not_conflict_with_completion(self):
        self.experiment = "controls"
        self.freeze()
        other = self.root / "worker"
        shutil.copytree(self.run, other)
        original = runner.run_role
        def snapshot(*args):
            run, manifest, task, configuration, case, area = args[:6]
            artifact = area / "progress.stdout"
            artifact.write_text("partial")
            runner.import_results(self.run, run)
            artifact.write_text("finished")
            return original(*args)
        with mock.patch.object(runner, "run_role", side_effect=snapshot):
            runner.worker(other, "local", 1)
        self.assertEqual(runner.import_results(self.run, other)["imported_tasks"], 1)
        row = next(iter(runner.latest(runner.journal(self.run)).values()))
        self.assertEqual(row["event"], "complete")
        self.assertEqual((self.run / "tasks" / row["task_id"] / "progress.stdout").read_text(), "finished")
