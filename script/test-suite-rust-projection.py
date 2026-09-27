"""AST-based, fail-closed projection of Linux Rust source into production lines.

This is a source projection, not a Rust build or a claim of executed coverage.
"""
import hashlib
import bisect
import itertools
import json
from pathlib import Path
import re

from tree_sitter import Language, Parser
import tree_sitter_rust

POLICY = {
    "version": 1,
    "parser": "tree-sitter=0.25.2,tree-sitter-rust=0.24.2",
    "target": "x86_64-unknown-linux-gnu",
    "test_only_features": ["test-util", "integration_tests"],
    "production_features": [],
    "excluded_paths": ["separate test files", "crates/integration", "build scripts", "generated target files"],
    "test_attributes": ["test", "tokio::test", "warpui::test", "test_case", "rstest", "proptest"],
}
POLICY_HASH = hashlib.sha256(json.dumps(POLICY, sort_keys=True).encode()).hexdigest()
COMMENT_TYPES = {"line_comment", "block_comment"}
TEST_PATH = re.compile(r"(^|/)(test|tests|mod_test)\.rs$|(_test|_tests)\.rs$|/tests/")
RUST_LANGUAGE = Language(tree_sitter_rust.language())


def cfg_values(text):
    """Possible values in production; unknown normal features remain unknown."""
    tokens = re.findall(r'"(?:\\.|[^"\\])*"|[A-Za-z_][A-Za-z_0-9]*|[(),=]', text)
    if re.sub(r'\s+', '', ''.join(tokens)) != re.sub(r'\s+', '', text):
        raise ValueError(f"unsupported cfg expression: {text}")
    index = 0

    def parse():
        nonlocal index
        if index >= len(tokens):
            raise ValueError("truncated cfg")
        key = tokens[index]
        index += 1
        if index < len(tokens) and tokens[index] == '(':
            index += 1
            values = []
            while index < len(tokens) and tokens[index] != ')':
                values.append(parse())
                if index < len(tokens) and tokens[index] == ',':
                    index += 1
                elif index < len(tokens) and tokens[index] != ')':
                    raise ValueError("invalid cfg arguments")
            if index == len(tokens):
                raise ValueError("unclosed cfg")
            index += 1
            if key == 'not' and len(values) == 1:
                return {not value for value in values[0]}
            if key in {'all', 'any'}:
                operation = all if key == 'all' else any
                return {operation(row) for row in itertools.product(*values)}
            raise ValueError(f"unsupported cfg operator: {key}")
        if index < len(tokens) and tokens[index] == '=':
            index += 1
            if index == len(tokens):
                raise ValueError("missing cfg value")
            value = json.loads(tokens[index])
            index += 1
            if key == 'feature' and value in POLICY['test_only_features']:
                return {False}
            if key == 'feature' and value in POLICY['production_features']:
                return {True}
            known = {'target_os': 'linux', 'target_family': 'unix', 'target_arch': 'x86_64', 'target_pointer_width': '64'}
            return {value == known[key]} if key in known else {False, True}
        known = {'test': False, 'unix': True, 'windows': False, 'debug_assertions': True}
        return {known[key]} if key in known else {False, True}

    result = parse()
    if index != len(tokens):
        raise ValueError("trailing cfg tokens")
    return result


def attribute_text(node):
    return node.named_children[0].text.decode()


def attribute_excludes(node):
    text = attribute_text(node)
    if text.split('(', 1)[0].strip() in POLICY['test_attributes']:
        return True
    if text.startswith('cfg('):
        if not re.search(r'\btest\b|"(?:test-util|integration_tests)"', text):
            return False
        values = cfg_values(text[4:-1])
        if values == {False}:
            return True
        if len(values) > 1 and re.search(r'\btest\b|"(?:test-util|integration_tests)"', text):
            raise ValueError(f"ambiguous production cfg: {text}")
    if text.startswith('cfg_attr('):
        # Lints do not change generated executable code. Other test-dependent
        # attributes (notably derive) need explicit region evidence from LLVM.
        if re.search(r'\btest\b|"(?:test-util|integration_tests)"', text) and not re.search(r',\s*(allow|warn|deny|forbid|expect)\(', text):
            raise ValueError(f"test-dependent generated code: {text}")
    return False


def project_source(source):
    tree = Parser(RUST_LANGUAGE).parse(source)
    if tree.root_node.has_error:
        raise ValueError("Rust AST contains an error or missing node")
    excluded = []
    comments = []
    modules = []
    diagnostics = []

    def walk(node, inline=(), inherited=False):
        pending = []
        for child in node.named_children:
            if child.type in COMMENT_TYPES:
                comments.append((child.start_byte, child.end_byte))
                continue
            if child.type == 'attribute_item':
                pending.append(child)
                continue
            if child.type == 'inner_attribute_item':
                if attribute_excludes(child):
                    excluded.append((node.start_byte, node.end_byte))
                    inherited = True
                continue
            try:
                # An enclosing cfg(test) proves the complete item test-only,
                # regardless of any derives or other attributes on that item.
                cfgs = [attr for attr in pending if attribute_text(attr).startswith('cfg(')]
                suppressed = inherited or any(attribute_excludes(attr) for attr in cfgs)
                suppressed = suppressed or any(attribute_excludes(attr) for attr in pending)
            except ValueError as error:
                diagnostics.append((child.start_point.row + 1, child.end_point.row + 1, str(error)))
                suppressed = inherited
            start = pending[0].start_byte if pending else child.start_byte
            if suppressed:
                end = child.end_byte
                separator = child.next_sibling
                if separator is not None and separator.type in {',', ';'}:
                    end = separator.end_byte
                excluded.append((start, end))
            if child.type == 'mod_item':
                name = child.child_by_field_name('name').text.decode()
                body = child.child_by_field_name('body')
                if body is None:
                    custom = None
                    for attr in pending:
                        value = attribute_text(attr)
                        if value.startswith('path'):
                            match = re.fullmatch(r'path\s*=\s*("(?:\\.|[^"\\])*")', value)
                            if match is None:
                                raise ValueError("unsupported module path attribute")
                            custom = json.loads(match[1])
                    modules.append((inline, name, custom, suppressed))
                else:
                    walk(body, (*inline, name), suppressed)
            elif child.type in {'macro_definition', 'macro_invocation'}:
                if not suppressed and re.search(rb'#\s*\[.*?(?:\btest\b|test-util|integration_tests)', child.text, re.S):
                    if child.type == 'macro_invocation' and child.child_by_field_name('macro').text in {b'cfg_if', b'cfg_if::cfg_if'}:
                        tokens = [part for part in child.named_children[-1].children[1:-1] if part.type not in COMMENT_TYPES]
                        possible = True
                        index = 0
                        while index < len(tokens):
                            first = tokens[index]
                            if first.text == b'else':
                                index += 1
                                first = tokens[index]
                            if first.text == b'if':
                                if index + 3 >= len(tokens) or tokens[index + 1].text != b'#':
                                    raise ValueError('unsupported cfg_if structure')
                                condition = tokens[index + 2].text.decode()
                                match = re.fullmatch(r'\[\s*cfg\((.*)\)\s*\]', condition, re.S)
                                if not match:
                                    raise ValueError('unsupported cfg_if condition')
                                values = cfg_values(match[1])
                                body = tokens[index + 3]
                                index += 4
                            else:
                                values, body = {True}, first
                                index += 1
                            if not body.text.startswith(b'{'):
                                raise ValueError('unsupported cfg_if body')
                            if not possible or values == {False}:
                                excluded.append((first.start_byte, body.end_byte))
                            elif re.search(rb'#\s*\[.*?(?:\btest\b|test-util|integration_tests)', body.text, re.S):
                                diagnostics.append((body.start_point.row + 1, body.end_point.row + 1, 'nested test-dependent macro requires expansion'))
                            if values == {True}:
                                possible = False
                    else:
                        diagnostics.append((child.start_point.row + 1, child.end_point.row + 1, 'test items generated inside an unexpanded macro'))
            elif not suppressed:
                walk(child, inline, False)
            pending = []
        if pending:
            raise ValueError("unattached Rust attributes")

    walk(tree.root_node)
    masked = bytearray(source)
    for start, end in excluded + comments:
        for index in range(start, end):
            if masked[index] not in (10, 13):
                masked[index] = 32
    lines = {}
    canonical = []
    for number, line in enumerate(bytes(masked).decode().splitlines(), 1):
        content = line.strip()
        if content:
            canonical.append(content)
            lines[number] = len(canonical)
    # LCOV merges all regions on one physical line. If a test-only item shares
    # that line with production, its hit cannot prove production execution.
    starts = [0] + [match.end() for match in re.finditer(b'\n', source)]
    for start, end in excluded:
        first = bisect.bisect_right(starts, start)
        last = bisect.bisect_right(starts, max(start, end - 1))
        for number in range(first, last + 1):
            if number in lines:
                diagnostics.append((number, number, 'test and production share one LCOV source line'))
    fingerprint = hashlib.sha256('\n'.join(canonical).encode()).hexdigest()
    return lines, fingerprint, modules, diagnostics


class Projection:
    def __init__(self, root):
        self.root = root.resolve()
        self.cache = {}
        self.line_counts = {}
        self.errors = {}
        self.test_files = set()
        self.production_files = set()

    def analyze(self, relative):
        if relative not in self.cache:
            path = (self.root / relative).resolve()
            if not path.is_relative_to(self.root) or not path.is_file():
                raise ValueError(f"missing or external Rust source: {relative}")
            try:
                data = path.read_bytes()
                self.line_counts[relative] = len(data.splitlines())
                self.cache[relative] = project_source(data)
            except ValueError as error:
                raise ValueError(f'{relative}: {error}') from error
        return self.cache[relative]

    def index_modules(self, roots):
        pending = [(str(path), False) for path in roots]
        visited = set()
        while pending:
            relative, excluded = pending.pop()
            if (relative, excluded) in visited:
                continue
            visited.add((relative, excluded))
            (self.test_files if excluded else self.production_files).add(relative)
            # Test-only subtrees have no production metrics and need not be parsed.
            if excluded:
                continue
            if TEST_PATH.search(relative) or relative.startswith('crates/integration/'):
                self.test_files.add(relative)
                self.production_files.discard(relative)
                continue
            try:
                _, _, modules, _ = self.analyze(relative)
            except ValueError as error:
                self.errors[relative] = str(error)
                continue
            path = Path(relative)
            default_base = path.parent if path.name in {'lib.rs', 'main.rs', 'mod.rs'} else path.with_suffix('')
            for inline, name, custom, suppressed in modules:
                if custom is not None:
                    possibilities = [(default_base if inline else path.parent).joinpath(*inline, custom)]
                else:
                    base = default_base.joinpath(*inline)
                    possibilities = [base / f'{name}.rs', base / name / 'mod.rs']
                existing = [item for item in possibilities if (self.root / item).is_file()]
                # Platform/config-conditional modules may not exist in this target.
                if not existing:
                    continue
                if len(existing) != 1:
                    raise ValueError(f"ambiguous external Rust module: {relative}:{name}")
                child = existing[0].as_posix()
                pending.append((child, suppressed))
                if suppressed:
                    # Descendant source files remain test-only unless also reached
                    # through a production module; no filename guessing is needed.
                    directory = existing[0].parent if existing[0].name == 'mod.rs' else existing[0].with_suffix('')
                    if (self.root / directory).is_dir():
                        self.test_files.update(p.relative_to(self.root).as_posix() for p in (self.root / directory).rglob('*.rs'))

    def line(self, relative, number):
        if relative in self.test_files and relative not in self.production_files:
            return None
        if TEST_PATH.search(relative) or relative.startswith('crates/integration/'):
            # Conventional test filenames are a declared exclusion policy, not
            # inferred executed-test savings.
            return None
        if relative not in self.production_files:
            raise ValueError(f'Rust source has no proven production module route: {relative}')
        mapping, fingerprint, _, diagnostics = self.analyze(relative)
        for start, end, reason in diagnostics:
            if start <= number <= end:
                raise ValueError(f'{relative}:{number}: {reason}')
        if number > self.line_counts[relative]:
            raise ValueError(f"LCOV line outside source: {relative}:{number}")
        ordinal = mapping.get(number)
        return (f'{relative}:{ordinal}', fingerprint) if ordinal is not None else None


def crate_roots(root):
    # Conventional roots and explicitly named src/bin targets used by this
    # workspace. Never promote each LCOV source file to a crate root.
    roots = []
    packages = [root / 'app', *sorted((root / 'crates').glob('*'))]
    for package in packages:
        for name in ('src/lib.rs', 'src/main.rs'):
            path = package / name
            if path.is_file():
                roots.append(path.relative_to(root))
        for path in sorted((package / 'src/bin').glob('*.rs')):
            roots.append(path.relative_to(root))
    return roots


def audit_sources(root):
    projection = Projection(root)
    projection.index_modules(crate_roots(projection.root))
    diagnostics = {
        path: [{'start': first, 'end': last, 'reason': reason} for first, last, reason in rows]
        for path, (_, _, _, rows) in projection.cache.items() if rows
    }
    return projection, {
        'policy': POLICY,
        'policy_sha256': POLICY_HASH,
        'production_files': len(projection.production_files),
        'test_only_files': len(projection.test_files - projection.production_files),
        'ast_errors': projection.errors,
        'unresolved_regions': diagnostics,
        'status': 'needs-review' if projection.errors or diagnostics else 'pass',
        'note': 'Static source classification only; no tests or coverage measurement executed.',
    }


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    _, report = audit_sources(args.root)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
