#!/usr/bin/env bash
# WS2 i18n guard (RC master plan) — flag bare user-facing string literals in the
# text render constructors on the primary surfaces that have been fully
# localized, so they can't silently regress back to hard-coded English copy.
#
# Deliberately scoped: only the surfaces WS2 finished are guarded. The deep
# inherited-Warp long tail, the git-dialog loading labels still behind
# &'static str APIs, and the settings-group descriptions (which are
# &'static str in the settings *schema* — a separate framework task) are NOT
# guarded here. Extend FILES as more surfaces are localized.
set -euo pipefail
cd "$(dirname "$0")/.."

FILES=(
  app/src/tab_configs/session_config_modal.rs
  app/src/tab_configs/new_worktree_modal.rs
  app/src/workspace/native_modal.rs
)

# Scan whole files so a newline before the literal cannot bypass the guard.
# Python's regular expressions also work on hosts without GNU grep/PCRE.
python3 - "${FILES[@]}" <<'PYTHON'
from pathlib import Path
import re
import sys

pattern = re.compile(r'(Text::new|Text::new_inline|FormattedTextElement::from_str|Span::new)\(\s*"')
hits = []
for name in sys.argv[1:]:
    source = Path(name).read_text(encoding="utf-8")
    for match in pattern.finditer(source):
        line = source.count("\n", 0, match.start()) + 1
        hits.append(f"{name}:{line}: {match.group(1)}")
if hits:
    print("i18n guard FAILED — bare user-facing literal(s) in a render path")
    print("(route them through crate::t! / t_static! + warp.ftl):")
    print("\n".join(hits))
    raise SystemExit(1)
print("i18n guard OK — no bare literals in render constructors on the guarded surfaces.")
PYTHON
