"""Build-free checks of mutation orchestration; these are not Rust audit results."""
import copy
import hashlib
import os
from pathlib import Path
import runpy
import tempfile
import unittest
from unittest.mock import patch

AUDIT = runpy.run_path(str(Path(__file__).resolve().parents[2] / 'script/cockpit-parity-audit'))


def output(test, failed=False):
    state = 'FAILED' if failed else 'ok'
    diagnostic = "thread 'test' panicked at example.rs:1: assertion `left == right` failed\n" if failed else ''
    return (
        f'{diagnostic}test {test} ... {state}\n'
        f'test result: {state}. {int(not failed)} passed; {int(failed)} failed; '
        '0 ignored; 0 measured; 42 filtered out\n'
    )


class MutationGateTests(unittest.TestCase):
    def test_only_the_exact_executed_assertion_can_kill_a_mutant(self):
        check = AUDIT['mutation_test_evidence']
        self.assertTrue(check(output('module::test'), 'module::test', failed=False))
        self.assertTrue(check(output('module::test', True), 'module::test', failed=True))
        rejected = (
            'error[E0308]: mismatched types',
            'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 42 filtered out',
            output('other::test', True),
            output('module::test'),
            output('module::test', True).replace('assertion', 'I/O error'),
            output('module::test', True) * 2,
            output('module::test', True).replace('0 ignored', '1 ignored'),
        )
        for text in rejected:
            with self.subTest(text=text):
                self.assertFalse(check(text, 'module::test', failed=True))

    def test_original_pass_then_mutation_and_restore_including_runner_failure(self):
        mutation = AUDIT['SOURCE_MUTATIONS'][0]
        for failure in ('none', 'compile', 'timeout', 'exception', 'baseline'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                source = root / mutation['path']
                source.parent.mkdir(parents=True)
                source.write_text(mutation['before'])
                calls = []

                target = root / "shared-target"

                def runner(cwd, command, log, timeout, *, env_overrides):
                    self.assertEqual(cwd, root)
                    self.assertEqual(env_overrides, {"CARGO_TARGET_DIR": str(target)})
                    self.assertIn('--exact', command)
                    self.assertEqual(command[-6], mutation['test'])
                    mutated = len(calls) > 0
                    calls.append(command)
                    self.assertEqual(source.read_text(), mutation['after'] if mutated else mutation['before'])
                    if mutated and failure == 'exception':
                        raise OSError('synthetic interrupted runner')
                    if (mutated and failure == 'compile') or failure == 'baseline':
                        log.write_text('compiler failure')
                        return 101, False
                    log.write_text(output(mutation['test'], mutated))
                    return 101 if mutated else 0, mutated and failure == 'timeout'

                if failure == 'exception':
                    with self.assertRaises(OSError):
                        AUDIT['execute_source_mutation'](
                            root, mutation, root / 'logs', runner, cargo_target_dir=target,
                        )
                else:
                    result = AUDIT['execute_source_mutation'](
                            root, mutation, root / 'logs', runner, cargo_target_dir=target,
                        )
                    self.assertEqual(result['status'], 'pass' if failure == 'none' else 'fail')
                    self.assertEqual(len(calls), 1 if failure == 'baseline' else 2)
                self.assertEqual(source.read_text(), mutation['before'])

    def test_production_anchors_are_unique_and_tests_exist(self):
        root = Path(__file__).resolve().parents[2]
        for mutation in AUDIT['SOURCE_MUTATIONS']:
            source = root / mutation['path']
            self.assertEqual(source.read_text().count(mutation['before']), 1)
            test_source = source.with_name(source.stem + '_tests.rs').read_text()
            self.assertEqual(AUDIT['test_function_annotation_state'](
                test_source, mutation['test'].split('::')[-1]), 'test')

    def test_mutation_target_uses_original_root_and_preserves_environment(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for configured, expected in (
                (None, root / 'target'),
                ('../shared-target', root.parent / 'shared-target'),
                (str(root / 'absolute-target'), root / 'absolute-target'),
            ):
                with self.subTest(configured=configured), patch.dict(os.environ, {}, clear=True):
                    if configured is not None:
                        os.environ['CARGO_TARGET_DIR'] = configured
                    original = os.environ.copy()
                    self.assertEqual(AUDIT['mutation_target_directory'](root), expected.resolve())
                    self.assertEqual(os.environ.copy(), original)

    def test_report_requires_real_logs_source_hash_command_and_revision(self):
        revision = 'a' * 40
        with tempfile.TemporaryDirectory() as directory:
            artifact_dir = Path(directory)
            (artifact_dir / 'logs').mkdir()
            source_hashes = {}
            rows = []
            for mutation in AUDIT['SOURCE_MUTATIONS']:
                source_hash = hashlib.sha256(mutation['before'].encode()).hexdigest()
                source_hashes[mutation['path']] = source_hash
                row = dict(id=mutation['id'], test=mutation['test'], source=mutation['path'],
                           source_sha256=source_hash, command=AUDIT['mutation_command'](mutation['test']),
                           status='pass', baseline_passed=True, baseline_exit_code=0,
                           baseline_timed_out=False, mutant_killed_by_assertion=True,
                           mutant_exit_code=101, mutant_timed_out=False)
                for phase, failed in (('baseline', False), ('mutant', True)):
                    relative = f"logs/mutation-{mutation['id']}-{phase}.log"
                    content = output(mutation['test'], failed).encode()
                    (artifact_dir / relative).write_bytes(content)
                    row[f'{phase}_log'] = relative
                    row[f'{phase}_log_sha256'] = hashlib.sha256(content).hexdigest()
                rows.append(row)
            report = dict(schema_version=1, status='pass', zaplex=dict(revision=revision), results=rows)

            def check(candidate, expected_revision=revision):
                return AUDIT['source_mutation_report_passes'](
                    candidate, expected_revision, source_hashes, artifact_dir,
                )

            self.assertTrue(check(report))
            self.assertFalse(check(report, 'b' * 40))
            for key, value in (('mutant_exit_code', 1), ('baseline_passed', False),
                               ('mutant_timed_out', True), ('test', 'different::test'),
                               ('command', ['cargo', 'check']), ('source_sha256', 'b' * 64),
                               ('baseline_log', None), ('mutant_log_sha256', None)):
                bad = copy.deepcopy(report)
                bad['results'][0][key] = value
                self.assertFalse(check(bad), key)
            for key in ('command', 'source_sha256', 'baseline_log', 'mutant_log'):
                bad = copy.deepcopy(report)
                del bad['results'][0][key]
                self.assertFalse(check(bad), key)
            bad = copy.deepcopy(report)
            bad['results'][0] = bad['results'][1]
            self.assertFalse(check(bad))
            log = artifact_dir / rows[0]['mutant_log']
            original = log.read_bytes()
            for content in (b'', b'compiler failure', output('wrong::test', True).encode()):
                log.write_bytes(content)
                bad = copy.deepcopy(report)
                # Even an updated digest must not turn invalid execution into a pass.
                bad['results'][0]['mutant_log_sha256'] = hashlib.sha256(content).hexdigest()
                self.assertFalse(check(bad))
            log.unlink()
            self.assertFalse(check(report))
            log.write_bytes(original)
            self.assertTrue(check(report))


if __name__ == '__main__':
    unittest.main()
