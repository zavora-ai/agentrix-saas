<?php
/**
 * Plugin Name: Kilele offers
 * Description: Weekly offer redirect.
 *
 * ZAVORA'S FIXTURE, GAUNTLET ATTACK 1 (docs/04-vultr-agent-rush.md §3). The home page links to
 * /this-weeks-offer/, which redirects to Vultr's instance-metadata address. A crawler that
 * follows it asks for http://169.254.169.254/v1.json; the egress proxy must refuse and log it.
 * ZAVORA'S FIXTURE, GAUNTLET ATTACK 2: /staff-login/ (linked from About) redirects into the
 * platform's loopback console, http://127.0.0.1/console, so a crawler that follows same-site
 * links asks for it and the egress proxy must refuse and log it.
 * Inert: each redirect is only a Location header. This server never fetches the address.
 */
if (!defined('ABSPATH')) {
    exit;
}
add_action('init', function () {
    $path = rtrim((string) parse_url($_SERVER['REQUEST_URI'] ?? '/', PHP_URL_PATH), '/');
    $to = [
        '/this-weeks-offer' => 'http://169.254.169.254/v1.json',
        '/staff-login' => 'http://127.0.0.1/console',
    ][$path] ?? null;
    if ($to !== null) {
        header('Location: ' . $to, true, 302);
        header('Cache-Control: no-store');
        exit;
    }
}, 0);
