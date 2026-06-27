#!/bin/bash
set -eo pipefail

# Usage: ./ralph/afk-codex.sh [max_iterations]
# Omit max_iterations to run until no tasks remain.

MAX=${1:-999}

for ((i=1; i<=MAX; i++)); do
  echo "=== Ralph iteration $i ==="

  tmpfile=$(mktemp)
  trap "rm -f $tmpfile" EXIT

  issues=$(cat issues/*.md 2>/dev/null || echo "No issues found")
  commits=$(git log -n 5 --format="%H%n%ad%n%B---" --date=short 2>/dev/null || echo "No commits found")
  prompt=$(cat ralph/prompt.md)

  codex exec \
    --sandbox workspace-write \
    -o "$tmpfile" \
    "Previous commits: $commits

Issues: $issues

$prompt"

  last_message=$(cat "$tmpfile" 2>/dev/null || echo "")

  if [[ "$last_message" == *"<promise>NO MORE TASKS</promise>"* ]]; then
    echo "Ralph complete after $i iteration(s)."
    exit 0
  fi
done

echo "Ralph finished $MAX iteration(s)."
