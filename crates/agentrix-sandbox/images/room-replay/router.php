<?php
// room-replay's router for PHP's built-in server: real files are served as themselves (and PHP
// files run, as on the old host); every other path goes through WordPress, so pretty
// permalinks work without rewrite rules.
$root = realpath($_SERVER['DOCUMENT_ROOT']);
$path = urldecode(parse_url($_SERVER['REQUEST_URI'], PHP_URL_PATH) ?? '/');
$file = realpath($root . $path);
if ($file !== false && is_file($file) && strncmp($file, $root . '/', strlen($root) + 1) === 0) {
    return false;
}
$_SERVER['SCRIPT_NAME'] = '/index.php';
$_SERVER['SCRIPT_FILENAME'] = $root . '/index.php';
require $root . '/index.php';
