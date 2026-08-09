#!/usr/bin/env bash
# Generate GitHub release notes from the commit range since the previous
# version tag. Groups commits by conventional-commit prefix (feat, fix,
# docs, chore, refactor, test, ci, perf, build, style, deps, other).
#
# Usage:
#   scripts/release-notes.sh <release-tag>            # infers previous tag
#   scripts/release-notes.sh <release-tag> <prev-tag> # explicit previous tag
set -euo pipefail

release="${1:?usage: release-notes.sh <release-tag> [prev-tag]}"
prev="${2:-}"

if [ -z "$prev" ]; then
  # Highest existing v* tag strictly BELOW the release tag, so backfilled
  # older releases diff against their real predecessor rather than the
  # newest tag. Assumes plain vX.Y.Z tags (no prerelease suffixes).
  prev="$(git tag -l 'v*' --sort=-version:refname \
    | sort -V \
    | grep -oE '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    | awk -v rel="${release}" 'rel > $0 { p=$0 } END { if (p != "") print p }' || true)"
fi

echo "## What's Changed"
echo ""
if [ -z "$prev" ]; then
  echo "Initial release of **flo-rs ${release}**."
else
  echo "Changes since **${prev}**:"
fi
echo ""

if [ -z "$prev" ]; then
  range="HEAD"
else
  range="${prev}..HEAD"
fi

group() {
  local label="$1"
  local pattern="$2"
  local lines
  lines=$(git log --no-merges --pretty=format:'%s' "${range}" \
    | sed -n "s/^${pattern}: \(.*\)/- \1/p" || true)
  if [ -n "$lines" ]; then
    echo "### ${label}"
    echo ""
    printf '%s\n' "$lines"
    echo ""
  fi
}

group "Features" "feat"
group "Fixes" "fix"
group "Documentation" "docs"
group "Under the hood" "refactor"
group "Performance" "perf"
group "Dependencies" "deps|build\\(deps\\)"
group "Build & CI" "build|ci"
group "Chores" "chore"
group "Tests" "test"

others=$(git log --no-merges --format=format:'%s' "${range}" | grep -vEv '^(feat|fix|docs|refactor|perf|chore|test|deps|build|ci)(\([^)]*\))?:' || true)
if [ -n "$others" ]; then
  echo "### Other changes"
  echo ""
  printf '%s\n' "$others"
fi

echo ""
echo "**Full Changelog**: \`${prev:-N/A}\` → \`${release}\`"
echo ""
echo "Checksums: see the attached SBOM (\`sbom.cdx.json\`) for dependency verification."