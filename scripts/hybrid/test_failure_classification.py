"""Budget interruptions and declared control failures have distinct outcomes."""
import time
import unittest
from unittest import mock
import test_runner as fixtures

runner = fixtures.runner


class FailureClassificationTests(unittest.TestCase):
    setUp = fixtures.RunnerTests.setUp
    tearDown = fixtures.RunnerTests.tearDown
    freeze = fixtures.RunnerTests.freeze

    def test_input_deadline_is_not_changed_input(self):
        m = self.freeze()
        with mock.patch.object(runner, "snapshot_tree", side_effect=runner.TaskInterrupted("deadline")):
            with self.assertRaises(runner.TaskInterrupted):
                runner.sample(self.run, m, runner.tasks(m, "screen")[0], time.monotonic() + 10)

    def test_native_deadline_is_not_control_failure(self):
        self.experiment = "controls"
        m = self.freeze()
        def role(*args):
            if args[9] == "native":
                raise runner.TaskInterrupted("deadline")
        with mock.patch.object(runner, "run_role", side_effect=role):
            with self.assertRaises(runner.TaskInterrupted):
                runner.sample(self.run, m, runner.tasks(m, "screen")[0], time.monotonic() + 10)

    def test_declared_hybrid_control_failure_is_fatal(self):
        m = self.freeze()
        for index, role_name in enumerate(("relationships", "streams", "native_ordered")):
            task = runner.tasks(m, "screen")[index]
            def role(*args):
                if args[9] == role_name:
                    raise runner.RunError("broken control")
            with self.subTest(role=role_name), \
                 mock.patch.object(runner, "run_role", side_effect=role), \
                 mock.patch.object(runner, "archive_order", return_value=[]):
                with self.assertRaises(runner.SampleError) as caught:
                    runner.sample(self.run, m, task, time.monotonic() + 10)
                self.assertTrue(caught.exception.control_failure)

    def test_interrupted_task_does_not_poison_run(self):
        self.experiment = "controls"
        self.freeze()
        with mock.patch.object(runner, "sample", side_effect=runner.TaskInterrupted("deadline")):
            runner.worker(self.run, "local", 1)
        states = runner.latest(runner.journal(self.run))
        self.assertEqual([row["event"] for row in states.values()], ["interrupted"])
        self.assertFalse(any(row.get("control_failure") for row in states.values()))
        runner.worker(self.run, "local")
        self.assertEqual(runner.report(self.run)["tasks"]["interrupted"], 1)

    def test_candidate_order_failure_is_not_control_failure(self):
        m = self.freeze()
        with mock.patch.object(runner, "run_role"), \
             mock.patch.object(runner, "archive_order", side_effect=runner.RunError("candidate listing")):
            with self.assertRaises(runner.RunError) as caught:
                runner.sample(self.run, m, runner.tasks(m, "screen")[0], time.monotonic() + 10)
        self.assertFalse(getattr(caught.exception, "control_failure", False))

    def test_actual_child_timeout_remains_a_sample_failure(self):
        import sys
        self.experiment = "controls"
        self.freeze(task_timeout_seconds=.1, budgets={"screen": 10, "confirm": 10})
        def sample(run, manifest, task, deadline):
            runner.execute([sys.executable, "-c", "import time; time.sleep(5)"],
                           self.root, self.root, "slow-child", deadline, {})
        with mock.patch.object(runner, "sample", side_effect=sample):
            runner.worker(self.run, "local", 1)
        row = next(iter(runner.latest(runner.journal(self.run)).values()))
        self.assertEqual(row["event"], "failed")
        self.assertFalse(row["control_failure"])
        self.assertFalse(hasattr(runner.EXECUTION_CONTEXT, "task_limited"))

    def test_real_sample_native_timeout_does_not_poison_run(self):
        self.experiment = "controls"
        self.freeze(task_timeout_seconds=.5, budgets={"screen": 10, "confirm": 10},
                    worker_options={"native_add": ["--fake-sleep"]})
        runner.worker(self.run, "local", 1)
        row = next(iter(runner.latest(runner.journal(self.run)).values()))
        self.assertEqual(row["event"], "failed")
        self.assertFalse(row["control_failure"])

    def test_input_task_timeout_does_not_claim_corruption(self):
        m = self.freeze()
        with mock.patch.object(runner, "snapshot_tree", side_effect=runner.TaskTimeout("deadline")):
            with self.assertRaises(runner.TaskTimeout):
                runner.sample(self.run, m, runner.tasks(m, "screen")[0], time.monotonic() + 10)
