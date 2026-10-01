#!/usr/bin/env bash
# Usage: scripts/worktree.sh <name> <branch> [base=origin/main]
# Makes ../simple-editor-wt/<name> with its OWN target/ dir, pre-seeded by hard-linking the main
# tree's compiled dependencies (no copy, ~0 disk). Each worktree then builds only its own crate,
# incrementally, with no lock shared with other worktrees: ~20 s per change instead of queueing
# behind every other worktree's full crate rebuild on one shared target dir.
# Also usable on an existing worktree: scripts/worktree.sh --seed <path>
set -e
main=$(git rev-parse --path-format=absolute --git-common-dir)/..
seed() {
  src="$main/target/debug" dst="$1/target/debug"
  [ -d "$dst/deps" ] && return 0
  [ -d "$src/deps" ] || { echo "no $src yet: run 'cargo test --no-run' in the main tree once"; return 0; }
  mkdir -p "$dst/deps" "$dst/build" "$dst/.fingerprint"
  find "$src/deps" -maxdepth 1 -type f ! -name '*simple_editor*' -exec cp -l {} "$dst/deps/" \;
  for d in build .fingerprint; do
    find "$src/$d" -mindepth 1 -maxdepth 1 ! -name 'simple-editor-*' -exec cp -rl {} "$dst/$d/" \;
  done
}
if [ "$1" = --seed ]; then seed "$2"; exit; fi
git -C "$main" fetch -q origin
git -C "$main" worktree add "$main/../simple-editor-wt/$1" -b "$2" "${3:-origin/main}"
seed "$main/../simple-editor-wt/$1"
