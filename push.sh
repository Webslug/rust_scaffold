#!/usr/bin/env bash
set -euo pipefail


# Usage:
#   ./push.sh
#   ./push.sh "Describe this change"
#


WORK_DIR="${WORK_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
REPO_URL="${REPO_URL:-https://github.com/Webslug/rust_scaffold.git}"
BRANCH="${BRANCH:-main}"
COMMIT_MESSAGE="${1:-Rust Scaffold update $(date '+%Y-%m-%d %H:%M:%S')}"

if [ ! -d "$WORK_DIR" ]; then
    echo "[ERROR] Project folder does not exist: $WORK_DIR"
    exit 1
fi

cd "$WORK_DIR"

if [ ! -d .git ]; then
    echo "[INFO] Initialising Git repository..."
    git init
fi

git branch -M "$BRANCH"

if git remote get-url origin >/dev/null 2>&1; then
    git remote set-url origin "$REPO_URL"
else
    git remote add origin "$REPO_URL"
fi

echo "[INFO] Staging project files..."
git add -A

if git diff --cached --quiet; then
    echo "[INFO] Nothing new to commit."
else
    git commit -m "$COMMIT_MESSAGE"
fi

# --force-with-lease lets this local project remain authoritative, but refuses
# to overwrite a remote commit that appeared after the last time Git checked.
echo "[INFO] Pushing to GitHub..."
git push -u origin "$BRANCH" --force-with-lease

echo "[DONE] Rust Scaffold project pushed to GitHub."
