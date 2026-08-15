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

# Text/Span constructors whose first argument is user-visible: a string literal
# there bypasses crate::t! / t_static!. Matches `Ctor("…` or `Ctor(\n  "…`.
PATTERN='(Text::new|Text::new_inline|FormattedTextElement::from_str|Span::new)\(\s*"'

hits="$(grep -rnzoP "$PATTERN" "${FILES[@]}" 2>/dev/null | tr '\0' '\n' || true)"
# Fallback simple line grep (portable): catch the common single-line form too.
hits2="$(grep -rnE "$PATTERN" "${FILES[@]}" 2>/dev/null || true)"

if [[ -n "$hits2" ]]; then
  echo "i18n guard FAILED — bare user-facing literal(s) in a render path"
  echo "(route them through crate::t! / t_static! + warp.ftl):"
  echo "$hits2"
  exit 1
fi

echo "i18n guard OK — no bare literals in render constructors on the guarded surfaces."
