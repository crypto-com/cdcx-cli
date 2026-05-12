#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

VERSION="${1:-}"

if [ -z "$VERSION" ]; then
  CURRENT=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version = "\([^"]*\)"/\1/p;}' "$REPO_ROOT/Cargo.toml")
  MAJOR=$(echo "$CURRENT" | cut -d. -f1)
  MINOR=$(echo "$CURRENT" | cut -d. -f2)
  PATCH=$(echo "$CURRENT" | cut -d. -f3)
  VERSION="$MAJOR.$MINOR.$((PATCH + 1))"
  echo "No version specified, bumping patch: $CURRENT → $VERSION"
fi

if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "Error: Invalid version format. Expected semver (e.g. 1.3.0)"
  exit 1
fi

cd "$REPO_ROOT"

echo "Bumping version to $VERSION..."

# Update Cargo.toml workspace version
sed -i '' "/^\[workspace\.package\]/,/^\[/ s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml

# Update Cargo.lock
cargo update -w

# Update package.json version
jq --arg v "$VERSION" '.version = $v' package.json > package.json.tmp
mv package.json.tmp package.json

# Update plugin.json versions
for f in plugins/cdcx-cli/.claude-plugin/plugin.json \
         plugins/cdcx-cli/.codex-plugin/plugin.json \
         plugins/cdcx-cli/.cursor-plugin/plugin.json; do
  if [ -f "$f" ]; then
    jq --arg v "$VERSION" '.version = $v' "$f" > "$f.tmp"
    mv "$f.tmp" "$f"
  fi
done

# Commit, tag, and push
git add Cargo.toml Cargo.lock package.json plugins/
git commit -m "chore(release): bump version to $VERSION"
git push

echo "Done — v$VERSION pushed."
