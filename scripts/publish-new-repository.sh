#!/usr/bin/env bash
set -euo pipefail

repo="rwjblue/slotpilot"
description="Attended weak-signal operation for FT8 and WSPR"

if ! command -v gh >/dev/null 2>&1; then
  echo "GitHub CLI (gh) is required." >&2
  exit 1
fi

if ! gh auth status >/dev/null 2>&1; then
  echo "Authenticate GitHub CLI first with: gh auth login" >&2
  exit 1
fi

if gh repo view "$repo" >/dev/null 2>&1; then
  echo "$repo already exists; refusing to create or overwrite it." >&2
  exit 1
fi

if [[ ! -d .git ]]; then
  git init -b main
fi

git add -A
if git diff --cached --quiet; then
  echo "No staged changes to publish." >&2
  exit 1
fi

git commit -m "Initialize SlotPilot design scaffold"
gh repo create "$repo"   --public   --description "$description"   --source .   --remote origin   --push

echo "Published https://github.com/$repo"
