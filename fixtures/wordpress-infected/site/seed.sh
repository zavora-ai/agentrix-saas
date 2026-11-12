#!/bin/sh
# Seeds Kilele Bakery (fictional), Zavora's fixture, in the `cli` container (wp-cli, user 33).
# FIXTURE_SIZE=dev  -> 5 pages  (4 pages + 1 post): development Moves (design D10c)
# FIXTURE_SIZE=demo -> 50 pages (10 pages + 40 posts): acceptance, the gauntlet and the demo
# Offline: no plugin or theme is downloaded. Adapted from adk-rust examples/website seed.sh.
set -eu
cd /var/www/html
SIZE=${FIXTURE_SIZE:-dev}
case $SIZE in dev|demo) ;; *) echo "FIXTURE_SIZE must be dev or demo" >&2; exit 2;; esac
ORIGIN=${FIXTURE_ORIGIN:-https://kilele-bakery.example}

if wp core is-installed 2>/dev/null; then
  echo "already seeded (run ./down.sh first to change size)"; exit 0
fi
# Fake, fixture-only admin credential, labelled as such. Never a real password.
wp core install --url="$ORIGIN" --title="Kilele Bakery" --admin_user=owner \
  --admin_password=fixture-only-not-a-real-password --admin_email=owner@kilele-bakery.example \
  --skip-email
wp option update blogdescription "Bread, pastries and celebration cakes, baked on Ngong Road since 2016"
wp option update timezone_string "Africa/Nairobi"
wp option update date_format "j F Y"
wp option update kl_fixture_size "$SIZE"
wp rewrite structure '/%postname%/' --hard

# The site's own files: theme and the base mu-plugin.
mkdir -p wp-content/mu-plugins wp-content/themes
cp /fixture/site/mu-plugins/fixture.php wp-content/mu-plugins/
rm -rf wp-content/themes/kilele
cp -r /fixture/site/theme/kilele wp-content/themes/kilele
(cd wp-content/themes/kilele && sh build.sh)
wp theme activate kilele
# Only our theme and no bundled plugins, so wp-content stays small and self-contained.
wp plugin delete akismet hello >/dev/null 2>&1 || true
for t in $(wp theme list --field=name --status=inactive); do wp theme delete "$t" >/dev/null; done

# Start from nothing: drop the sample post, page and privacy draft.
for id in $(wp post list --post_type=post,page --post_status=any --field=ID); do
  wp post delete "$id" --force >/dev/null
done

# Media, at fixed upload paths.
UPLOADS=wp-content/uploads/2026/10
mkdir -p "$UPLOADS"
cp /fixture/site/uploads/2026/10/*.png "$UPLOADS/"
wp media import "$UPLOADS/celebration-cake.png" --skip-copy --title="Celebration cake" \
  --alt="A three-layer celebration cake with pink icing" >/dev/null
wp media import "$UPLOADS/sourdough-loaf.png" --skip-copy --title="Sourdough loaf" \
  --alt="A scored sourdough loaf" >/dev/null
wp media import "$UPLOADS/kilele-team.png" --skip-copy --title="The Kilele team" \
  --alt="Three bakers in aprons" >/dev/null

wp eval-file /fixture/site/seed.php "$SIZE"
wp rewrite flush --hard
echo "seeded: $SIZE"
