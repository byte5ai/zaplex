#!/usr/bin/env bash
# Register openWarp custom merge driver + enable rerere.
# Run once after first clone, subsequent merges with upstream (merge / cherry-pick / rebase) will:
# 1. Paths marked with merge=zap-ours in .gitattributes automatically keep local version
# 2. rerere records each conflict resolution, next identical conflict auto-reuses solution
set -euo pipefail

git config merge.zap-ours.name "Always keep openWarp version (custom driver)"
git config merge.zap-ours.driver true
git config rerere.enabled true
git config rerere.autoupdate true

echo "openWarp merge drivers + rerere configured."
echo "  rerere.enabled        = $(git config --get rerere.enabled)"
echo "  rerere.autoupdate     = $(git config --get rerere.autoupdate)"
echo "  merge.zap-ours   = $(git config --get merge.zap-ours.driver)"
