#!/bin/bash
# room-replay (S2-05; design §3.1; R4.1-R4.3): replays an old WordPress site sealed, from its
# database dump (and its wp-content, so its own plugins and infections run) or a WXR export,
# and records what it serves.
#
# Room contract (agentrix-runner):
#   /in/job.json   {"public_origin": "https://kilele.example",
#                   "dump": "<sha256 hex>"         SQL dump, plain or gzip
#                     or "wxr": "<sha256 hex>",    WXR export
#                   "wp_content": "<sha256 hex>",  optional tar(.gz) of wp-content
#                   "max_pages": 200}
#   /in/<hex>      the inputs, read-only
#   /out           inventory.json, recording.json, site/{browser,crawler}/NNNN.html,
#                  rest/*.json, files.json, logs.txt
#   stdout         {"event":"note","text":"..."} lines
#
# The site believes it is public_origin but is served on 127.0.0.1 only. Every HTTP request its
# PHP makes (WordPress's HTTP API, PHP streams, cURL) goes to the job's egress proxy, whose
# allowlist is empty: the request is refused and recorded on the job's receipt. Anything else
# has no route out of the room.
set -euo pipefail
umask 077

IN=/in
OUT=/out
WORK=/tmp/replay
WP=$WORK/wp
DB=$WORK/db
SOCK=$WORK/db.sock
PORT=8080
LOCAL="http://127.0.0.1:$PORT"
BROWSER_UA='Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36'
CRAWLER_UA='Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)'

note() { jq -cn --arg t "$1" '{event: "note", text: $t}'; }
fail() {
  note "replay failed: $1"
  exit 1
}
# A sha256 hex digest from job.json, or nothing.
hexarg() {
  local v
  v=$(jq -r --arg k "$1" '.[$k] // empty' "$IN/job.json")
  if [[ -n "$v" && ! "$v" =~ ^[0-9a-f]{64}$ ]]; then fail "$1 is not a sha256 hex digest"; fi
  printf '%s' "$v"
}
# Loopback only: never through the proxy.
fetch() { curl --noproxy '*' --silent --show-error --max-time 20 "$@"; }
# GET $2 as user agent $1 into /out/$3; prints the HTTP status as a number (0 if none).
get() {
  local code
  code=$(fetch -A "$1" -o "$OUT/$3" -w '%{http_code}' "$2" 2>/dev/null) || true
  code=${code: -3}
  [[ $code =~ ^[0-9]{3}$ ]] || code=000
  echo $((10#$code))
}

[[ -f $IN/job.json ]] || fail "no /in/job.json"
ORIGIN=$(jq -r '.public_origin // empty' "$IN/job.json")
[[ "$ORIGIN" =~ ^https?://[A-Za-z0-9.-]+(:[0-9]+)?/?$ ]] || fail "public_origin must be an http(s) origin"
ORIGIN=${ORIGIN%/}
SCHEME=${ORIGIN%%://*}
HOSTPORT=${ORIGIN#*://}
DUMP=$(hexarg dump)
WXR=$(hexarg wxr)
CONTENT=$(hexarg wp_content)
MAX_PAGES=$(jq -r '.max_pages // 200' "$IN/job.json")
[[ "$MAX_PAGES" =~ ^[0-9]+$ ]] || MAX_PAGES=200
((MAX_PAGES <= 500)) || MAX_PAGES=500
[[ -n "$DUMP" || -n "$WXR" ]] || fail "give a dump or a wxr"
for d in $DUMP $WXR $CONTENT; do [[ -f $IN/$d ]] || fail "input $d is missing"; done

mkdir -p "$WORK" "$OUT/site/browser" "$OUT/site/crawler" "$OUT/rest"

# 1. MariaDB, on a socket only.
note "starting the database"
mariadb-install-db --no-defaults --datadir="$DB" --auth-root-authentication-method=normal \
  --skip-test-db >"$WORK/db-init.log" 2>&1 || fail "the database would not initialise"
mariadbd --no-defaults --datadir="$DB" --socket="$SOCK" --pid-file="$WORK/db.pid" \
  --skip-networking --tmpdir=/tmp --innodb-buffer-pool-size=128M \
  --innodb-use-native-aio=0 >"$WORK/db.log" 2>&1 & # gVisor has no Linux native AIO
sql() { mariadb --no-defaults --socket="$SOCK" -uroot "$@"; }
for _ in $(seq 60); do
  mariadb-admin --no-defaults --socket="$SOCK" -uroot ping >/dev/null 2>&1 && break
  sleep 0.5
done
mariadb-admin --no-defaults --socket="$SOCK" -uroot ping >/dev/null 2>&1 || fail "the database did not start"
sql -e 'CREATE DATABASE wp CHARACTER SET utf8mb4'

# 2. WordPress core, writable, and the site's own wp-content.
cp -r --preserve=mode,timestamps /opt/wordpress "$WP"
if [[ -n "$CONTENT" ]]; then
  note "unpacking wp-content"
  mkdir "$WORK/content"
  tar --no-same-owner --no-same-permissions -xf "$IN/$CONTENT" -C "$WORK/content" ||
    fail "wp_content is not a tar archive"
  src="$WORK/content"
  [[ -d $src/wp-content ]] && src="$src/wp-content"
  rm -rf "$WP/wp-content"
  mv "$src" "$WP/wp-content"
fi
mkdir -p "$WP/wp-content/plugins" "$WP/wp-content/uploads"

PREFIX=wp_
if [[ -n "$DUMP" ]]; then
  if gzip -t "$IN/$DUMP" 2>/dev/null; then
    gunzip -c "$IN/$DUMP" >"$WORK/dump.sql"
  else
    cp "$IN/$DUMP" "$WORK/dump.sql"
  fi
  # shellcheck disable=SC2016 # literal backticks, as in SQL
  found=$(grep -m1 -oE 'CREATE TABLE `[A-Za-z0-9_]*options`' "$WORK/dump.sql" || true)
  found=${found#CREATE TABLE \`}
  found=${found%options\`}
  [[ "$found" =~ ^[A-Za-z0-9_]+$ ]] && PREFIX=$found
fi

# The job's proxy, from the runner's environment.
PROXY=${http_proxy:-${HTTP_PROXY:-}}
proxy_lines=""
if [[ "$PROXY" =~ ^http://([0-9.]+):([0-9]+)$ ]]; then
  proxy_lines="define('WP_PROXY_HOST', '${BASH_REMATCH[1]}'); define('WP_PROXY_PORT', '${BASH_REMATCH[2]}');
define('WP_PROXY_BYPASS_HOSTS', 'localhost,127.0.0.1');"
fi
https=off
[[ $SCHEME == https ]] && https=on
salt() { head -c 48 /dev/urandom | base64 | tr -dc 'A-Za-z0-9'; }

cat >"$WP/wp-config.php" <<PHP
<?php
// Written by room-replay. The site believes it is $ORIGIN; it is served on loopback only.
define('DB_NAME', 'wp');
define('DB_USER', 'root');
define('DB_PASSWORD', '');
define('DB_HOST', 'localhost:$SOCK');
define('DB_CHARSET', 'utf8mb4');
define('DB_COLLATE', '');
\$table_prefix = '$PREFIX';
\$_SERVER['HTTP_HOST'] = '$HOSTPORT';
\$_SERVER['HTTPS'] = '$https';
define('WP_HOME', '$ORIGIN');
define('WP_SITEURL', '$ORIGIN');
$proxy_lines
define('AUTOMATIC_UPDATER_DISABLED', true);
define('WP_AUTO_UPDATE_CORE', false);
define('DISALLOW_FILE_MODS', true);
define('WP_DEBUG', true);
define('WP_DEBUG_LOG', '$WORK/wp-debug.log');
define('WP_DEBUG_DISPLAY', false);
define('AUTH_KEY', '$(salt)');
define('SECURE_AUTH_KEY', '$(salt)');
define('LOGGED_IN_KEY', '$(salt)');
define('NONCE_KEY', '$(salt)');
define('AUTH_SALT', '$(salt)');
define('SECURE_AUTH_SALT', '$(salt)');
define('LOGGED_IN_SALT', '$(salt)');
define('NONCE_SALT', '$(salt)');
if (!defined('ABSPATH')) {
    define('ABSPATH', __DIR__ . '/');
}
require_once ABSPATH . 'wp-settings.php';
PHP

wp_cli() { wp --path="$WP" "$@"; }
# Reads that should not run the site's plugins or theme.
wpq() { wp --path="$WP" --skip-plugins --skip-themes "$@" 2>/dev/null; }

# 3. The content.
if [[ -n "$DUMP" ]]; then
  note "loading the database dump"
  sql wp <"$WORK/dump.sql" || fail "the dump did not load"
  MODE=dump
else
  note "importing the WXR export"
  cp -r /opt/replay-plugins/wordpress-importer "$WP/wp-content/plugins/"
  wp_cli core install --url="$ORIGIN" --title=Replay --admin_user=replay \
    --admin_password="$(salt)" --admin_email=replay@replay.invalid --skip-email >/dev/null ||
    fail "WordPress would not install"
  wp_cli plugin activate wordpress-importer >/dev/null || fail "the importer would not activate"
  wp_cli import "$IN/$WXR" --authors=create --skip=attachment >/dev/null || fail "the WXR did not import"
  MODE=wxr
fi

# 4. Serve on loopback.
note "serving the replayed site on loopback"
# Several workers, so WordPress's request to its own wp-cron.php does not wait on the page.
(cd "$WP" && PHP_CLI_SERVER_WORKERS=4 exec php -S "127.0.0.1:$PORT" -t "$WP" /opt/replay/router.php \
  >"$WORK/php-server.log" 2>&1) &
for _ in $(seq 60); do
  fetch -o /dev/null "$LOCAL/" 2>/dev/null && break
  sleep 0.5
done

# 5. The REST collections, as WordPress serves them.
note "reading the REST collections"
rest_files=()
for type in pages posts media categories tags; do
  for n in $(seq 1 20); do
    file="rest/$type-$n.json"
    code=$(get "$BROWSER_UA" "$LOCAL/?rest_route=/wp/v2/$type&per_page=100&page=$n" "$file")
    if [[ $code != 200 ]] || ! jq -e 'type == "array" and length > 0' "$OUT/$file" >/dev/null 2>&1; then
      rm -f "$OUT/$file"
      break
    fi
    rest_files+=("$file")
  done
done

# 6. The pages: once as a browser and once as a crawler, so cloaking shows.
paths=("/")
add_link() {
  if [[ $1 == "$ORIGIN"/* ]]; then paths+=("/${1#"$ORIGIN"/}"); fi
}
while IFS= read -r link; do add_link "$link"; done < <(
  for f in "$OUT"/rest/pages-*.json "$OUT"/rest/posts-*.json; do
    if [[ -f $f ]]; then jq -r '.[].link // empty' "$f"; fi
  done
)
while IFS= read -r link; do add_link "$link"; done < <(
  wpq menu list --field=term_id | while read -r menu; do wpq menu item list "$menu" --field=url || true; done
)
mapfile -t paths < <(printf '%s\n' "${paths[@]}" | awk '!seen[$0]++' | head -n "$MAX_PAGES")
note "replaying ${#paths[@]} pages"

: >"$WORK/recording.jsonl"
i=0
for path in "${paths[@]}"; do
  i=$((i + 1))
  id=$(printf '%04d' "$i")
  b="site/browser/$id.html"
  c="site/crawler/$id.html"
  code_b=$(get "$BROWSER_UA" "$LOCAL$path" "$b")
  code_c=$(get "$CRAWLER_UA" "$LOCAL$path" "$c")
  sum_b=$( (sha256sum "$OUT/$b" 2>/dev/null || true) | cut -d' ' -f1)
  sum_c=$( (sha256sum "$OUT/$c" 2>/dev/null || true) | cut -d' ' -f1)
  forms=$( (grep -oi '<form' "$OUT/$b" 2>/dev/null || true) | wc -l | tr -d ' ')
  jq -cn --arg path "$path" --arg url "$ORIGIN$path" --arg b "$b" --arg c "$c" \
    --arg sb "$sum_b" --arg sc "$sum_c" --argjson cb "$code_b" --argjson cc "$code_c" \
    --argjson forms "$((10#${forms:-0}))" \
    '{path: $path, public_url: $url, browser: {file: $b, status: $cb, sha256: $sb},
      crawler: {file: $c, status: $cc, sha256: $sc}, differs: ($sb != $sc), forms: $forms}' \
    >>"$WORK/recording.jsonl"
  if ((i % 10 == 0)); then note "page $i/${#paths[@]}: $path"; fi
done
jq -s . "$WORK/recording.jsonl" >"$OUT/recording.json"

# 7. The files the site carries, hashed, with their first bytes for type checks.
note "hashing wp-content"
(cd "$WP/wp-content" && find . -type f -size -50M | head -n 5000 | while IFS= read -r f; do
  jq -cn --arg p "${f#./}" --arg s "$(sha256sum "$f" | cut -d' ' -f1)" \
    --argjson n "$(stat -c %s "$f")" --arg m "$(head -c 8 "$f" | od -An -tx1 | tr -d ' \n')" \
    '{path: $p, sha256: $s, bytes: $n, magic: $m}'
done) | jq -s . >"$OUT/files.json"

# 8. The inventory.
note "writing the inventory"
logs() { cat "$WORK/php-server.log" "$WORK/wp-debug.log" /tmp/php-errors.log 2>/dev/null || true; }
logs | tail -c 200000 >"$OUT/logs.txt"
json() { "$@" --format=json 2>/dev/null || echo '[]'; }
jq -n \
  --arg origin "$ORIGIN" --arg mode "$MODE" --arg prefix "$PREFIX" \
  --arg version "$(wpq core version || echo unknown)" \
  --argjson pages "$(json wpq post list --post_type=page --post_status=publish --fields=ID,post_title,post_name,post_date)" \
  --argjson posts "$(json wpq post list --post_type=post --post_status=publish --fields=ID,post_title,post_name,post_date)" \
  --argjson media "$(json wpq post list --post_type=attachment --fields=ID,post_title,guid,post_mime_type)" \
  --argjson menus "$(json wpq menu list --fields=term_id,name,slug,count)" \
  --argjson cf7 "$(json wpq post list --post_type=wpcf7_contact_form --fields=ID,post_title)" \
  --argjson wpforms "$(json wpq post list --post_type=wpforms --fields=ID,post_title)" \
  --argjson plugins "$(json wpq plugin list --fields=name,status,version)" \
  --argjson themes "$(json wpq theme list --fields=name,status,version)" \
  --argjson mu "$(find "$WP/wp-content/mu-plugins" -mindepth 1 -maxdepth 1 -printf '%f\n' 2>/dev/null | jq -R . | jq -s .)" \
  --argjson rest "$(printf '%s\n' "${rest_files[@]+"${rest_files[@]}"}" | jq -R 'select(length > 0)' | jq -s .)" \
  --argjson recording "$(cat "$OUT/recording.json")" \
  --argjson egress "$(logs | grep -iE 'could not resolve|php_network_getaddresses|failed to connect|curl error|network is unreachable|connection refused' | tail -n 100 | jq -R . | jq -s .)" \
  '{public_origin: $origin, mode: $mode, wordpress: $version, table_prefix: $prefix,
    counts: {pages: ($pages | length), posts: ($posts | length), media: ($media | length),
             menus: ($menus | length), forms: (($cf7 | length) + ($wpforms | length)),
             pages_replayed: ($recording | length),
             cloaked: ([$recording[] | select(.differs)] | length)},
    pages: $pages, posts: $posts, media: $media, menus: $menus,
    forms: {contact_form_7: $cf7, wpforms: $wpforms,
            rendered: [$recording[] | select(.forms > 0) | {path, forms}]},
    plugins: $plugins, themes: $themes, mu_plugins: $mu,
    rest: $rest, recording: "recording.json", files: "files.json", logs: "logs.txt",
    egress_attempts_in_logs: $egress}' >"$OUT/inventory.json"

note "replay done: $(jq -r '.counts | "\(.pages) pages, \(.posts) posts, \(.media) media, \(.menus) menus, \(.forms) forms; \(.cloaked) of \(.pages_replayed) pages differ for crawlers"' "$OUT/inventory.json")"
