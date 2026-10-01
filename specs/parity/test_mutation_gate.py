"""Build-free checks of mutation orchestration; these are not Rust audit results."""
import copy
import hashlib
import json
import os
from pathlib import Path
import runpy
import struct
import zlib
import tempfile
import unittest
from types import SimpleNamespace
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


class AssemblyRevisionTests(unittest.TestCase):
    def test_old_suite_and_runtime_reports_cannot_certify_a_new_checkout(self):
        assemble = AUDIT['command_assemble']
        revision = 'a' * 40
        old_revision = 'b' * 40
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = SimpleNamespace(root=root, screenshot_dir=root / 'screenshots',
                                   output=root / 'report.json')
            args.screenshot_dir.mkdir()
            for name in AUDIT['EXPECTED_SCREENSHOTS']:
                (args.screenshot_dir / name).write_bytes(b'prevalidated mockup')
            reports = {
                'validation': {'status': 'pass', 'errors': []},
                'self_test': {'status': 'pass', 'cases': [
                    {'id': name, 'status': status}
                    for name, status in AUDIT['EXPECTED_SELF_TEST_RESULTS'].items()
                ]},
                'references': {'status': 'pass', 'audited_at': '2026-09-27T00:00:00Z',
                               'references': [
                    {'repository': repo, 'branch': branch, 'revision': revision}
                    for repo, branch in AUDIT['EXPECTED_REFERENCES'].values()
                ]},
                'mutations': {},
            }
            for name, value in reports.items():
                path = root / f'{name}.json'
                path.write_text(json.dumps(value))
                setattr(args, name, path)
            args.suites = root / 'suites.json'
            args.runtime = root / 'runtime.json'
            # Component validation has its own tests. Here it must not mask
            # assembly of individually passing reports from another checkout.
            with patch.dict(assemble.__globals__, {
                'git_revision': lambda _: {'revision': revision},
                'revision_source_hash': lambda *_: 'c' * 64,
                'source_mutation_report_passes': lambda *_: True,
            }), patch('builtins.print'):
                for suite_revision, runtime_revision, expected_suites, expected_runtime in (
                    (revision, revision, 'pass', 'pass'),
                    (old_revision, old_revision, 'fail', 'fail'),
                    (old_revision, revision, 'fail', 'pass'),
                    (revision, old_revision, 'pass', 'fail'),
                    (None, revision, 'fail', 'pass'),
                ):
                    with self.subTest(suite=suite_revision, runtime=runtime_revision):
                        args.suites.write_text(json.dumps({
                            'status': 'pass', 'zaplex': {'revision': suite_revision},
                            'results': [
                                {'id': name, 'status': 'pass', 'exit_code': 0,
                                 'evidence_complete': True, 'command': command}
                                for name, command in AUDIT['EXPECTED_SUITE_COMMANDS'].items()
                            ],
                        }))
                        args.runtime.write_text(json.dumps({
                            'schema_version': 1, 'status': 'pass', 'errors': [],
                            'zaplex_revision': runtime_revision,
                            'screenshots': [{'name': name}
                                            for name in AUDIT['EXPECTED_RUNTIME_SCREENSHOTS']],
                        }))
                        exit_code = assemble(args)
                        result = json.loads(args.output.read_text())
                        self.assertEqual(result['zaplex']['revision'], revision)
                        self.assertEqual(result['components']['suites'], expected_suites)
                        self.assertEqual(result['components']['manual_runtime'], expected_runtime)
                        self.assertEqual(exit_code == 0, expected_suites == 'pass')
                        self.assertEqual(result['release_gate_status'] == 'pass',
                                         expected_suites == expected_runtime == 'pass')


class RuntimePngTests(unittest.TestCase):
    @staticmethod
    def image(header, compressed, palettes=(), *, palette_after_data=False):
        chunk = AUDIT['png_chunk']
        palette_chunks = b''.join(chunk(b'PLTE', value) for value in palettes)
        data_chunks = (
            chunk(b'IDAT', b'') + chunk(b'IDAT', compressed[:1])
            + chunk(b'IDAT', compressed[1:])
        )
        return (
            b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', *header))
            + (data_chunks + palette_chunks if palette_after_data else palette_chunks + data_chunks)
            + chunk(b'IEND', b'')
        )

    def inspect(self, source):
        inspect = AUDIT['inspect_runtime_png']
        with tempfile.TemporaryDirectory() as directory, patch.dict(inspect.__globals__, {
            'MIN_RUNTIME_IMAGE_WIDTH': 1, 'MIN_RUNTIME_IMAGE_HEIGHT': 1,
        }):
            path = Path(directory) / 'capture.png'
            path.write_bytes(source)
            errors = []
            result = inspect(path, errors)
            return result, errors

    def test_png_legal_color_depth_and_interlace_modes(self):
        # Independently enumerated pass sizes for an 8x8 Adam7 image.
        adam7 = ((1, 1), (1, 1), (2, 1), (2, 2), (4, 2), (4, 4), (8, 4))
        formats = ((0, (1, 2, 4, 8, 16), 1), (2, (8, 16), 3),
                   (3, (1, 2, 4, 8), 1), (4, (8, 16), 2), (6, (8, 16), 4))
        for color, depths, channels in formats:
            for depth in depths:
                for interlace in (0, 1):
                    with self.subTest(color=color, depth=depth, interlace=interlace):
                        passes = adam7 if interlace else ((8, 8),)
                        pixels = b''.join(
                            bytes([row % 5]) + bytes((columns * channels * depth + 7) // 8)
                            for columns, rows in passes for row in range(rows)
                        )
                        palettes = (bytes(3),) if color in (2, 3, 6) else ()
                        result, errors = self.inspect(self.image(
                            (8, 8, depth, color, 0, 0, interlace), zlib.compress(pixels), palettes,
                        ))
                        self.assertIsNotNone(result)
                        self.assertEqual(errors, [])
        # Only the first Adam7 pass contains a pixel in a 1x1 image.
        result, errors = self.inspect(self.image((1, 1, 1, 0, 0, 0, 1), zlib.compress(b'\0\0')))
        self.assertIsNotNone(result)
        self.assertEqual(errors, [])

    def test_png_rejects_crc_valid_invalid_headers_and_palettes(self):
        header = (8, 8, 8, 2, 0, 0, 0)
        compressed = zlib.compress(bytes(8 * 25))
        cases = []
        for index, invalid in ((0, 0), (1, 0), (2, 4), (3, 1), (4, 1), (5, 1), (6, 2)):
            fields = list(header)
            fields[index] = invalid
            cases.append(self.image(fields, compressed))
        cases.extend((
            self.image((8, 8, 1, 3, 0, 0, 0), compressed),
            self.image((8, 8, 1, 3, 0, 0, 0), compressed, (bytes(9),)),
            self.image((8, 8, 8, 0, 0, 0, 0), compressed, (bytes(3),)),
            self.image(header, compressed, (b'',)),
            self.image(header, compressed, (bytes(4),)),
            self.image(header, compressed, (bytes(771),)),
            self.image(header, compressed, (bytes(3), bytes(3))),
            self.image(header, compressed, (bytes(3),), palette_after_data=True),
            self.image((16384, 16384, 16, 6, 0, 0, 0), compressed),
        ))
        for index, source in enumerate(cases):
            with self.subTest(index=index):
                result, errors = self.inspect(source)
                self.assertIsNone(result)
                self.assertTrue(errors)

    def test_png_rejects_crc_valid_broken_zlib_lengths_and_filters(self):
        header = (8, 8, 8, 2, 0, 0, 0)
        pixels = bytes(8 * 25)
        compressed = zlib.compress(pixels)
        cases = (
            b'not a zlib stream', compressed[:-1], compressed + b'trailing',
            compressed + compressed, zlib.compress(pixels[:-1]),
            zlib.compress(pixels + b'\0'), zlib.compress(b'\5' + pixels[1:]),
            zlib.compress(bytes(1024 * 1024)),
        )
        for index, payload in enumerate(cases):
            with self.subTest(index=index):
                result, errors = self.inspect(self.image(header, payload))
                self.assertIsNone(result)
                self.assertTrue(errors)

if __name__ == '__main__':
    unittest.main()
