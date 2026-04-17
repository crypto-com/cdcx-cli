#!/usr/bin/env bash
# Install cdcx git hooks into this clone. One-shot, idempotent.
# Uses `git config core.hooksPath` so no files are copied — the repo's
# hooks/ directory becomes the live hook source.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

chmod +x hooks/pre-commit hooks/pre-push
git config core.hooksPath hooks

echo "Installed hooks: pre-commit (fmt + clippy), pre-push (tests)"
echo "To uninstall: git config --unset core.hooksPath"
