#!/usr/bin/env bash
# Fail when public text contains machine paths, private names or internal
# labels. Scans tracked and untracked (not ignored) files, so the worktree and
# CI enforce the same rule.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
# Private words are assembled from parts so this script does not match itself.
private='mo''ss'
owner='eti''enne'
planning='rf''c'
paths='/home/[a-z]+/|/mnt/[a-z]'
names="${private}|${owner}|apo""llo|automation_game|Legion-Pro|\b(cla""ude|cod""ex)\b"
internal="\blanes?\b|\btask-[a-z0-9]{6,}|${planning} ?[0-9]|\bS[0-9]+ oracle\b|\bphase [0-9]+(\.[0-9x]+)?\b|\bP[0-9]b?\b"
pattern="${paths}|${names}|${internal}"
set +e
matches=$(git grep --untracked -niIE "$pattern" -- . ':!tools/check_public_text.sh')
status=$?
set -e
case "$status" in
  0) ;;
  1)
    echo 'PASS: public text contains no private machine paths, names or internal labels'
    exit 0
    ;;
  *) exit "$status" ;;
esac
printf '%s\n' "$matches"
echo 'FAIL: private machine paths, names or internal labels in public text' >&2
exit 1
