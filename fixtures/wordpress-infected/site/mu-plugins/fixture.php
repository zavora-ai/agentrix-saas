<?php
/**
 * Plugin Name: Zavora's fixture
 * Description: Serves Kilele Bakery (fictional) on loopback under its public origin, so a
 * request to 127.0.0.1:8081 is answered rather than redirected, and asks search engines not to
 * index this deliberately infected site. Adapted from adk-rust's examples/website fixture.
 */
if (!defined('ABSPATH')) {
    exit;
}
remove_action('template_redirect', 'redirect_canonical');
add_action('send_headers', function () {
    header('X-Robots-Tag: noindex, nofollow');
    header("X-Fixture: Zavora's fixture (Kilele Bakery is fictional)");
});
