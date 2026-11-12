#!/usr/bin/env bash
# Agentrix WebApp Platform · merge one workstream slice onto `platform`, checked (W0 only).
#
#   scripts/platform-merge.sh <branch> "<one-line summary>"
#       In the platform worktree: brings in `main` first, then merges <branch> without committing,
#       runs the whole workspace check, and commits the merge only if every check passes.
#       On any failure it aborts the merge and leaves `platform` exactly as it was.
#   scripts/platform-merge.sh --check
#       Runs the checks on `platform` as it stands.
#
# Checks: rustfmt on the Rust files the slice adds; clippy with -D warnings; cargo test; the copy lint when
# scripts/copy-lint exists. All with --locked against the shared build cache
# (specs/platform/how-we-work.md). Stops if the disk has under 15 GB free.
# The commit carries no AI attribution (organisation rule).
set -euo pipefail

PLATFORM_DIR=/Users/jameskaranja/Developer/hackathons/agentrix-saas
export CARGO_TARGET_DIR=/Users/jameskaranja/Developer/hackathons/.agentrix-saas-target
export CARGO_TERM_COLOR=never
# Build scripts and proc macros separated from other worktrees' too (scripts/wcargo).
export WCARGO_ISOLATE_BUILD=1

say() { printf '%s\n' "$*" >&2; }
die() { say "platform-merge: $*"; exit 1; }

cd "$PLATFORM_DIR"
[ "$(git rev-parse --abbrev-ref HEAD)" = platform ] || die "not on the platform branch"

free_gb() { df -g /System/Volumes/Data | awk 'NR==2 {print $4}'; }
# Twelve worktrees each link their own test binaries into the shared cache, about 10 GB
# together. Ones untouched for an hour are regenerable (cargo relinks a missing output), so
# prune them before judging the disk.
find "$CARGO_TARGET_DIR/debug/deps" -maxdepth 1 -type f -perm +111 ! -name '*.dylib' ! -name '*.so' \
  -mmin +60 -delete 2>/dev/null || true
[ "$(free_gb)" -ge 15 ] || die "only $(free_gb) GB free; the floor is 15 GB. Free space first."

# rustfmt on the Rust files this slice adds. The first release's files aren't formatted to the current
# rustfmt and belong to their owners, so a slice is judged on its new files only.
fmt_changed() {
  local files
  files=$(git diff --name-only --diff-filter=A "$1" -- '*.rs' || true)
  [ -z "$files" ] && { say "   no new Rust files"; return 0; }
  # shellcheck disable=SC2086
  rustfmt --edition 2024 --check $files
}

# Every step returns on failure by itself: `checks` runs inside `if !`, where bash ignores
# `set -e`, so without the `|| return 1` only the last step's status would count.
checks() {
  say "== fmt";    fmt_changed "$1" || return 1
  say "== clippy"; RUSTFLAGS="-D warnings" scripts/wcargo clippy --workspace --all-targets --locked || return 1
  say "== test";   scripts/wcargo test --workspace --locked || return 1
  if [ -x scripts/copy-lint ]; then
    say "== copy lint"
    python3 scripts/tests/test_copy_lint.py || return 1
    scripts/copy-lint || return 1
  fi
}

if [ "${1:-}" = --check ]; then
  checks main
  say "platform $(git rev-parse --short HEAD): all checks pass"
  exit 0
fi

branch=${1:?usage: scripts/platform-merge.sh <branch> "<summary>"}
summary=${2:?a one-line summary of what the slice adds}
[ -z "$(git status --porcelain --untracked-files=no)" ] || die "platform has uncommitted changes; commit or clear them first"
git rev-parse --verify --quiet "refs/heads/$branch" >/dev/null || die "no branch $branch"

before=$(git rev-parse HEAD)
if ! git merge-base --is-ancestor main HEAD; then
  say "== bringing in main"
  git merge --no-ff --no-edit -m "Merge main into platform" main || { git merge --abort; die "main does not merge cleanly into platform"; }
fi

say "== merging $branch"
if ! git merge --no-ff --no-commit "refs/heads/$branch"; then
  say "conflicts:"; git diff --name-only --diff-filter=U >&2
  git merge --abort
  die "$branch conflicts with platform; nothing changed beyond the main merge (now $(git rev-parse --short HEAD), was ${before:0:7})"
fi

if ! checks "$before"; then
  git merge --abort
  die "checks failed on $branch merged into platform; merge aborted"
fi

git commit -q -m "Merge $branch: $summary"
say "merged $branch onto platform as $(git rev-parse --short HEAD)"
