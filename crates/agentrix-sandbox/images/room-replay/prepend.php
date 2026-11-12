<?php
// room-replay: every PHP stream request goes to the job's egress proxy, whose allowlist is empty,
// so a replayed site's attempt to phone home is refused and recorded on the job's receipt.
// WordPress's HTTP API uses WP_PROXY_* (wp-config.php) and cURL reads http_proxy itself.
$proxy = getenv('http_proxy') ?: getenv('HTTP_PROXY');
if (is_string($proxy) && preg_match('#^http://([0-9.]+):([0-9]+)$#', $proxy, $m)) {
    stream_context_set_default([
        'http' => ['proxy' => "tcp://{$m[1]}:{$m[2]}", 'request_fulluri' => true],
    ]);
}
