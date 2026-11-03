"""A screening workload failure preserves other diagnostic workloads, not eligibility."""
import importlib.util
from pathlib import Path
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location(
    "hybrid_parallel_failure_fixture", Path(__file__).with_name("test_parallel.py"))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
runner = fixture.runner


class ScreenFailureTests(unittest.TestCase):
    setUp = fixture.ParallelScreenTests.setUp
    tearDown = fixture.ParallelScreenTests.tearDown
    freeze = fixture.ParallelScreenTests.freeze
    parallel_manifest = fixture.ParallelScreenTests.parallel_manifest
    receipt = fixture.ParallelScreenTests.receipt

    def policy_manifest(self, parallel=True):
        m = self.parallel_manifest(slots=2)
        m["execution"] = {"parallel_screening": parallel, "screen_failure_policy": "workload"}
        return m

    def test_policy_is_optional_and_only_accepts_declared_values(self):
        self.assertNotIn("screen_failure_policy", runner.execution_settings())
        for policy in ("configuration", "workload"):
            self.assertEqual(runner.execution_settings({"screen_failure_policy": policy})[
                "screen_failure_policy"], policy)
        for policy in (None, True, 1, [], {}, "skip", ""):
            with self.subTest(policy=policy), self.assertRaises(runner.RunError):
                runner.execution_settings({"screen_failure_policy": policy})

    def test_parallel_failure_keeps_remaining_workloads_and_resume_does_not_repeat(self):
        m = self.policy_manifest()
        attempted = []

        def sample(run, manifest, task, deadline):
            attempted.append(task)
            if task["case"] == "case-0":
                raise runner.RunError("fixture timeout")
            self.receipt(run, manifest, task, deadline)

        with mock.patch.object(runner, "sample", side_effect=sample):
            first = runner.parallel_screen(self.run, m, m["hosts"][0], 1)
            second = runner.parallel_screen(self.run, m, m["hosts"][0], 2)
        self.assertEqual(first["tasks_run"], 1)
        self.assertEqual(second["tasks_run"], 2)
        self.assertEqual(len({task["configuration"] for task in attempted}), 1)
        self.assertEqual(attempted, runner.tasks(m, "screen")[:3])
        rows = list(runner.latest(runner.journal(self.run)).values())
        self.assertEqual([row["event"] for row in rows], ["failed", "complete", "complete"])
        selection = runner.choose_finalists(self.run, m, runner.journal(self.run))
        self.assertEqual(selection["finalists"], [])
        self.assertEqual(selection["eligible_configurations"], 0)
        configuration = attempted[0]["configuration"]
        self.assertEqual(selection["coverage"][configuration]["verified_workloads"], ["case-1", "case-2"])
        diagnostics = runner.absolute_frontiers(runner.verified_results(self.run, runner.journal(self.run), "screen"))
        self.assertTrue(diagnostics)
        self.assertTrue(all("diagnostic" in record["scope"] for record in diagnostics.values()))

    def test_parallel_current_block_continues_after_failure(self):
        m = self.policy_manifest()
        attempted = []

        def sample(run, manifest, task, deadline):
            attempted.append(task)
            if task["case"] == "case-0":
                raise runner.RunError("fixture timeout")
            self.receipt(run, manifest, task, deadline)

        with mock.patch.object(runner, "sample", side_effect=sample):
            self.assertEqual(runner.parallel_screen(self.run, m, m["hosts"][0], 3)["tasks_run"], 3)
        self.assertEqual(attempted, runner.tasks(m, "screen")[:3])
        self.assertFalse(any(row.get("event") == "excluded" for row in runner.journal(self.run)))

    def test_serial_screening_continues_after_failure(self):
        m = self.policy_manifest(parallel=False)
        attempted = []

        def sample(run, manifest, task, deadline):
            attempted.append(task)
            if task["case"] == "case-0":
                raise runner.RunError("fixture timeout")
            self.receipt(run, manifest, task, deadline)

        with mock.patch.object(runner, "load", return_value=m), mock.patch.object(runner, "sample", side_effect=sample):
            result = runner.worker(self.run, "local", 3)
        self.assertEqual(result["tasks_run"], 3)
        self.assertEqual(attempted, runner.tasks(m, "screen")[:3])
        self.assertEqual(len({task["configuration"] for task in attempted}), 1)
        self.assertFalse(any(row.get("event") == "excluded" for row in runner.journal(self.run)))

    def test_confirmation_failure_still_excludes_configuration(self):
        m = self.policy_manifest(parallel=False)
        configuration = m["configurations"][0]["id"]
        runner.append(self.run, {"event": "phase-done", "phase": "screen", "host": "local"})
        runner.write_json(self.run / "selection.json", {
            "manifest_sha256": runner.sha(self.run / "manifest.json"), "finalists": [configuration]})
        actual_tasks = runner.tasks
        attempted = []

        def generated(manifest, phase, finalists=None):
            return [] if phase == "screen" else actual_tasks(manifest, phase, finalists)

        def sample(run, manifest, task, deadline):
            attempted.append(task)
            raise runner.RunError("fixture confirmation failure")

        with mock.patch.object(runner, "load", return_value=m), \
             mock.patch.object(runner, "tasks", side_effect=generated), \
             mock.patch.object(runner, "sample", side_effect=sample):
            runner.worker(self.run, "local")
        self.assertEqual(len(attempted), 1)
        self.assertEqual(attempted[0]["phase"], "confirm")
        rows = list(runner.latest(runner.journal(self.run)).values())
        self.assertEqual(sum(row["event"] == "failed" for row in rows), 1)
        self.assertGreater(sum(row["event"] == "excluded" for row in rows), 1)

    def test_control_failures_stop_workload_policy_in_both_execution_modes(self):
        m = self.policy_manifest()
        for parallel in (False, True):
            with self.subTest(parallel=parallel):
                m["execution"]["parallel_screening"] = parallel
                (self.run / "journal.jsonl").write_text("")
                attempted = []

                def sample(run, manifest, task, deadline):
                    attempted.append(task)
                    error = runner.RunError("fixture native control failure")
                    error.control_failure = True
                    raise error

                with mock.patch.object(runner, "load", return_value=m), mock.patch.object(runner, "sample", side_effect=sample):
                    with self.assertRaisesRegex(runner.RunError, "control failed"):
                        runner.worker(self.run, "local")
                self.assertTrue(attempted)
                self.assertEqual(len({task["configuration"] for task in attempted}), len(attempted))
                self.assertTrue(any(row.get("control_failure") for row in runner.journal(self.run)))
                with mock.patch.object(runner, "load", return_value=m), mock.patch.object(runner, "sample") as child:
                    with self.assertRaisesRegex(runner.RunError, "retained control failure"):
                        runner.worker(self.run, "local")
                child.assert_not_called()


if __name__ == "__main__":
    unittest.main()
