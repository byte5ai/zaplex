#!/usr/bin/env bash
# i18n DE↔EN parity guard (RC master plan).
#
# The German bundle (app/i18n/de/warp.ftl) is intentionally a subset of the
# English source: the deep inherited-Warp long tail is left on EN fallback for
# now. But the *user-facing chrome* the German UI actually shows — cockpit,
# the new-session menu, terminal input hints, the server file browser, the git
# dialog — MUST be fully translated, or the UI mixes DE and EN "wild
# durcheinander" (the exact regression that prompted this guard).
#
# This check FAILS only when a key under one of CRITICAL_PREFIXES exists in EN
# but is missing in DE. The remaining EN-only long tail is reported as an
# informational count, not a failure. Extend CRITICAL_PREFIXES as more surfaces
# are localized. Complements check-i18n-literals.sh (bare-literal guard).
set -euo pipefail
cd "$(dirname "$0")/.."
# Byte-order collation so `sort` and `comm` agree (locale-independent).
export LC_ALL=C

EN=app/i18n/en/warp.ftl
DE=app/i18n/de/warp.ftl

# Message-id lines only: `key = value` at column 0 (skip comments, blanks,
# indented attribute/continuation lines).
keys() { grep -oE '^[a-z0-9][a-z0-9_-]* =' "$1" | sed 's/ =$//' | sort -u; }

keys "$EN" > /tmp/i18n_en_keys.$$
keys "$DE" > /tmp/i18n_de_keys.$$
# EN-only = present in EN, absent in DE.
comm -23 /tmp/i18n_en_keys.$$ /tmp/i18n_de_keys.$$ > /tmp/i18n_missing.$$

# Surfaces that must be fully German (the user-facing chrome).
CRITICAL_PREFIXES=(
  cockpit-
  workspace-new-session
  workspace-new-worktree-config
  workspace-new-tab-config
  workspace-reopen-closed-session
  workspace-favorites
  workspace-favorite-
  workspace-left-panel
  terminal-input-
  terminal-zero-state-
  server-file-browser-menu-
  git-dialog-
)

critical_missing=""
while IFS= read -r key; do
  for p in "${CRITICAL_PREFIXES[@]}"; do
    case "$key" in
      "$p"*) critical_missing+="  $key"$'\n'; break ;;
    esac
  done
done < /tmp/i18n_missing.$$

total_missing=$(wc -l < /tmp/i18n_missing.$$ | tr -d ' ')
rm -f /tmp/i18n_en_keys.$$ /tmp/i18n_de_keys.$$ /tmp/i18n_missing.$$

echo "i18n parity: ${total_missing} EN key(s) not yet in DE (long tail on EN fallback — informational)."

if [[ -n "$critical_missing" ]]; then
  echo
  echo "i18n parity FAILED — user-facing chrome key(s) missing in DE (would mix DE/EN):"
  printf '%s' "$critical_missing"
  echo
  echo "Add German translations to $DE (or narrow CRITICAL_PREFIXES if a key is not user-facing)."
  exit 1
fi

echo "i18n parity OK — all critical user-facing surfaces are fully German."
