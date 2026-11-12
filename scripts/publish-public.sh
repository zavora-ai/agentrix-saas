#!/usr/bin/env bash
# Publish a commit of this (private) repository to the public GitHub repository as one new,
# ordinary commit. The private history never travels: the public repo only ever sees exported
# trees.
#
#   scripts/publish-public.sh [REF]            # dry run: export, scan, show what would change
#   scripts/publish-public.sh [REF] --yes      # also commit in the public checkout and push
#
# REF defaults to `main`. The public checkout defaults to `agentrix-saas-public` beside this
# repository's main checkout; set AGENTRIX_PUBLIC_DIR to use another folder. It must be a git
# checkout whose `origin` is the public repository (AGENTRIX_PUBLIC_REMOTE, default
# github.com/zavora-ai/agentrix-saas), with nothing uncommitted.
#
# What it checks before anything is copied:
# - the export holds no .env, deploy/.state, *.pem, *.key, *.db or objects.json;
# - no file is over 5 MB (the films and other renders are git-ignored);
# - gitleaks finds nothing (or, without gitleaks, no known key shape matches);
# - no value of 20 characters or more from this repository's local .env files appears in it.
# Any finding stops the run. It prints file names and rule names, never a matched value.
#
# It never pushes a branch or remote of this repository, and never rewrites public history.
# A Cargo build folder (target/) in the public checkout is left alone: it is git-ignored there.
set -euo pipefail

ref=main
yes=0
for arg in "$@"; do
  case "$arg" in
    --yes) yes=1 ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
    -*) echo "unknown option: $arg" >&2; exit 2 ;;
    *) ref=$arg ;;
  esac
done

die() { echo "publish-public: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "needs $1"; }
need git; need rsync; need jq

repo=$(git rev-parse --show-toplevel)
common=$(cd "$repo" && cd "$(git rev-parse --git-common-dir)" && pwd)
main_checkout=$(dirname "$common")
public=${AGENTRIX_PUBLIC_DIR:-$(dirname "$main_checkout")/agentrix-saas-public}
remote_want=${AGENTRIX_PUBLIC_REMOTE:-github.com/zavora-ai/agentrix-saas}

sha=$(git -C "$repo" rev-parse --verify "$ref^{commit}") || die "no commit named $ref"
short=$(git -C "$repo" rev-parse --short "$sha")

[ -d "$public/.git" ] || die "$public is not a git checkout (clone the public repo there first)"
origin=$(git -C "$public" remote get-url origin 2>/dev/null) || die "$public has no origin"
case "$origin" in *"$remote_want"*) ;; *) die "$public's origin is $origin, not $remote_want" ;; esac
[ -z "$(git -C "$public" status --porcelain)" ] || die "$public has uncommitted changes"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export_dir=$tmp/export
mkdir -p "$export_dir"
git -C "$repo" archive "$sha" | tar -x -C "$export_dir"
echo "exported $ref ($short): $(find "$export_dir" -type f | wc -l | tr -d ' ') files"

# Left out of the public repository, each by exact path: too large for it and published elsewhere.
leave_out=(
  "specs/submission/video/agentrix-vultr-demo.mp4"   # the earlier entry's film (26 MB), on YouTube
)
for f in "${leave_out[@]}"; do
  [ -f "$export_dir/$f" ] && { rm -f "$export_dir/$f"; echo "  left out: $f"; }
done

findings=0
flag() { echo "  FOUND: $*"; findings=$((findings + 1)); }

echo "== files that must never be published"
while IFS= read -r f; do flag "${f#"$export_dir"/}"; done < <(
  find "$export_dir" \( -name '.env' -o -path '*/deploy/.state*' -o -name '*.pem' -o -name '*.key' \
    -o -name '*.db' -o -name '*.db-wal' -o -name 'objects.json' \) -print)

echo "== files over 5 MB"
# Deliberate large files, each by exact path and capped at 15 MB.
big_ok=(
  "specs/submission/deck/agentrix-vultr-deck.pdf"   # the submission's 19-slide deck
)
while IFS= read -r f; do
  rel=${f#"$export_dir"/}
  allowed=0
  for ok in "${big_ok[@]}"; do [ "$rel" = "$ok" ] && allowed=1; done
  if [ "$allowed" -eq 1 ] && [ -z "$(find "$f" -size +15360k)" ]; then
    echo "  allowed: $rel ($(du -h "$f" | cut -f1))"
  else
    flag "$rel is over 5 MB"
  fi
done < <(find "$export_dir" -type f -size +5120k)

echo "== secret scan"
if command -v gitleaks >/dev/null 2>&1; then
  gitleaks dir "$export_dir" --redact --no-banner --log-level error \
    --report-format json --report-path "$tmp/gitleaks.json" --exit-code 0
  # Known false positives:
  # - `runuser -u agentrix-egress -- curl …` reads as a curl password;
  # - a test in agentrix-concepts writes a made-up key ("vk-secret-" and six digits) to check that
  #   a leak is caught. It is allowed at that exact line only, so if the line moves it is flagged
  #   again and someone looks;
  # - the did:key specification's own Ed25519 example (saas-badge), a made-up access token in a
  #   mocked token endpoint (saas-federation), and two PayPal sandbox approval links in the
  #   evidence files. Each is allowed at its exact line.
  jq -r --arg root "$export_dir/" '.[]
      | (.File | ltrimstr($root)) as $f
      | select((($f == "deploy/smoke/sandbox-rooms.sh" and .RuleID == "curl-auth-user")
                or ($f == "crates/agentrix-concepts/src/runner.rs" and .RuleID == "generic-api-key"
                    and .StartLine == 319)
                or ($f == "crates/saas-badge/src/resolve.rs" and .RuleID == "generic-api-key"
                    and .StartLine == 488)
                or ($f == "crates/saas-federation/src/tests.rs" and .RuleID == "generic-api-key"
                    and .StartLine == 582)
                or ($f == "deploy/saas/era/EVIDENCE.md" and .RuleID == "generic-api-key"
                    and .StartLine == 59)
                or ($f == "specs/saas/EVIDENCE.md" and .RuleID == "generic-api-key"
                    and .StartLine == 77)) | not)
      | "\($f):\(.StartLine) \(.RuleID)"' "$tmp/gitleaks.json" > "$tmp/leaks.txt"
  while IFS= read -r line; do flag "gitleaks: $line"; done < "$tmp/leaks.txt"
else
  echo "  gitleaks is not installed; checking known key shapes instead"
  patterns=(
    'sk-ant-[A-Za-z0-9_-]{10,}'
    '\bre_[A-Za-z0-9]{6,}_[A-Za-z0-9]{10,}'
    '\b[0-9]{8,10}:AA[A-Za-z0-9_-]{30,}'
    'BEGIN [A-Z ]*PRIVATE KEY'
    'whsec_[A-Za-z0-9]{10,}'
    '\bgh[pousr]_[A-Za-z0-9]{30,}'
    '\b[A-Z0-9]{36}\b'
    'AGE-SECRET-KEY-1'
  )
  for p in "${patterns[@]}"; do
    while IFS= read -r f; do flag "${f#"$export_dir"/} matches $p"; done < <(grep -rIlE -- "$p" "$export_dir" || true)
  done
fi

echo "== values from local .env files"
for envfile in "$main_checkout/.env" "$main_checkout/deploy/.env" "$repo/.env" "$repo/deploy/.env"; do
  [ -f "$envfile" ] || continue
  while IFS= read -r line; do
    name=${line%%=*}
    value=${line#*=}
    value=${value#\"}; value=${value%\"}; value=${value#\'}; value=${value%\'}
    [ "${#value}" -ge 20 ] || continue
    # Settings that are public by nature: host names, model names and service addresses.
    case "$name" in
      SAAS_HOST|SAAS_*_HOST|*_BASE_URL|AGENTRIX_TIER_*|AGENTRIX_EMBED|AGENTRIX_RERANK|AGENTRIX_SCREEN_*|AGENTRIX_STT|AGENTRIX_TTS|AGENTRIX_IMAGE) continue ;;
    esac
    if grep -rqF -- "$value" "$export_dir"; then flag "the value of $name from $envfile"; fi
  done < <(grep -E '^[A-Za-z_][A-Za-z0-9_]*=' "$envfile" || true)
done

[ "$findings" -eq 0 ] || die "$findings finding(s); nothing was copied"
echo "  clean"

echo "== changes against the public repo"
rsync -a --checksum --delete --exclude '/.git' --exclude '/target/' --dry-run --itemize-changes "$export_dir/" "$public/" \
  | awk '$1 ~ /^\*deleting/ {d++} $1 ~ /^>f\+/ {a++} $1 ~ /^>f[^+]/ {c++}
         END {printf "  %d added, %d changed, %d deleted\n", a, c, d}'

if [ "$yes" -ne 1 ]; then
  echo "dry run: nothing copied, committed or pushed. Run again with --yes to publish."
  exit 0
fi

rsync -a --checksum --delete --exclude '/.git' --exclude '/target/' "$export_dir/" "$public/"
git -C "$public" add -A
if git -C "$public" diff --cached --quiet; then
  echo "the public repo already matches $ref ($short); nothing to publish"
  exit 0
fi
git -C "$public" commit -q -m "Update from $ref ($short)"
git -C "$public" push origin HEAD
echo "published $ref ($short) as $(git -C "$public" rev-parse --short HEAD)"
