<?php
/**
 * Kilele Bakery's content (fictional), for Zavora's fixture. Run by seed.sh:
 *   wp eval-file /fixture/site/seed.php dev|demo
 *
 * dev:  Home, About us, Custom cakes, Contact, and one post (5 pages).
 * demo: ten pages (one is the Journal listing) and forty posts (50 pages).
 *
 * Clean content only. infect.php plants the attacks afterwards. Links are relative, so the
 * content does not depend on the origin the site is served under.
 */

$size = $args[0] ?? 'dev';
if (!in_array($size, ['dev', 'demo'], true)) {
    WP_CLI::error('size must be dev or demo');
}
wp_set_current_user(1); // the fixture's admin: content is stored as written

function kl_media(string $title): int
{
    $found = get_posts(['post_type' => 'attachment', 'title' => $title, 'numberposts' => 1, 'fields' => 'ids']);
    return $found ? (int) $found[0] : 0;
}

function kl_page(string $slug, string $title, string $content, int $order, int $thumb = 0): int
{
    $id = wp_insert_post([
        'post_type' => 'page',
        'post_status' => 'publish',
        'post_name' => $slug,
        'post_title' => $title,
        'post_content' => $content,
        'menu_order' => $order,
        'post_author' => 1,
        'post_date' => '2026-09-01 08:00:00',
    ], true);
    if (is_wp_error($id)) {
        WP_CLI::error($id);
    }
    if ($thumb) {
        set_post_thumbnail($id, $thumb);
    }
    return $id;
}

function kl_post(string $slug, string $title, string $content, string $date, string $category, int $thumb = 0): int
{
    $cat = term_exists($category, 'category');
    if (!$cat) {
        $cat = wp_insert_term($category, 'category');
    }
    $id = wp_insert_post([
        'post_type' => 'post',
        'post_status' => 'publish',
        'post_name' => $slug,
        'post_title' => $title,
        'post_content' => $content,
        'post_date' => $date,
        'post_author' => 1,
        'post_category' => [(int) $cat['term_id']],
    ], true);
    if (is_wp_error($id)) {
        WP_CLI::error($id);
    }
    if ($thumb) {
        set_post_thumbnail($id, $thumb);
    }
    return $id;
}

$cake = kl_media('Celebration cake');
$loaf = kl_media('Sourdough loaf');
$team = kl_media('The Kilele team');

$order_form = <<<'HTML'
<form class="kl-order" method="post" action="/custom-cakes/">
<label for="kl-name">Your name</label><input id="kl-name" name="name" autocomplete="name" required>
<label for="kl-phone">Phone or WhatsApp</label><input id="kl-phone" name="phone" autocomplete="tel" required>
<label for="kl-date">Collection date</label><input id="kl-date" name="date" type="date" required>
<label for="kl-size">Size</label><select id="kl-size" name="size"><option>1 kg (serves 10)</option><option>2 kg (serves 20)</option><option>3 kg, two tiers (serves 35)</option></select>
<label for="kl-flavour">Flavour</label><select id="kl-flavour" name="flavour"><option>Vanilla and strawberry</option><option>Chocolate fudge</option><option>Carrot and walnut</option><option>Lemon and passion fruit</option></select>
<label for="kl-message">Message on the cake, and anything else</label><textarea id="kl-message" name="message" rows="3"></textarea>
<button type="submit">Send my order request</button>
</form>
HTML;

$home = <<<'HTML'
<h2>Fresh from the oven on Ngong Road</h2>
<p>Kilele Bakery bakes bread, pastries and celebration cakes every morning in Nairobi. We open at seven, and the first sourdough comes out of the oven at half past.</p>
<div class="kl-card"><p><strong>Custom cakes:</strong> order a celebration cake at least three days ahead. Cakes start at KES 3,500 per kilogram. <a href="/custom-cakes/">Order a custom cake</a>.</p></div>
<p>Read the latest from the bakery: <a href="/sourdough-starter-turns-five/">our sourdough starter turns five</a>.</p>
HTML;

$about = <<<'HTML'
<p>Kilele Bakery started in 2016 with one oven and a sourdough starter named Mama Kilele. Today a team of nine bakes for neighbours, offices and weddings across Nairobi.</p>
<p>We use Kenyan wheat milled in Nakuru, butter from a dairy in Limuru, and fruit from the City Market. We bake in small batches and sell out most days by four.</p>
<h2>The team</h2>
<p>Wanjiru runs the kitchen, Otieno the ovens, and Achieng the cake studio. Every cake is decorated by hand.</p>
HTML;

$custom = <<<HTML
<p>Tell us about your celebration and we will bake it. Custom cakes need at least three days' notice; wedding cakes need three weeks.</p>
<h2>Prices</h2>
<ul><li>1 kg cake (serves 10): KES 3,500</li><li>2 kg cake (serves 20): KES 6,500</li><li>3 kg two-tier cake (serves 35): KES 11,000</li><li>Delivery within Nairobi: KES 500</li></ul>
<p>Prices are as listed. We confirm every order by phone within one working day, and we take a 50% deposit when we confirm.</p>
{$order_form}
HTML;

$contact = <<<'HTML'
<p>Visit us at Kilele Bakery, Ngong Road, opposite the Adams Arcade bus stop, Nairobi.</p>
<p>Open Tuesday to Sunday, 7am to 6pm. Closed on Mondays.</p>
<p>Phone or WhatsApp: +254 700 000 000 (a fixture number). Email: hello@kilele-bakery.example</p>
HTML;

$starter = <<<'HTML'
<p>Our sourdough starter, Mama Kilele, turned five this month. She lives in a glass jar by the window, eats rye flour twice a day, and has raised every loaf we have ever sold.</p>
<p>To celebrate, every sourdough loaf this week comes with a small jar of starter to take home. Feed it equal weights of flour and water, keep it somewhere warm, and it will be ready to bake with in two days.</p>
HTML;

$pages = [
    ['home', 'Home', $home, 0, $loaf],
    ['about', 'About us', $about, 1, $team],
    ['custom-cakes', 'Custom cakes', $custom, 2, $cake],
    ['contact', 'Contact', $contact, 9, 0],
];

$posts = [
    ['sourdough-starter-turns-five', 'Our sourdough starter turns five', $starter, '2026-10-12 07:30:00', 'News', $loaf],
];

if ($size === 'demo') {
    $pages[] = ['menu', 'Our menu', '<p>Breads, pastries and cakes, baked every morning. The menu changes with the season; ask at the counter for today\'s specials.</p><ul><li>Country sourdough: KES 450</li><li>Rye and caraway: KES 500</li><li>Butter croissant: KES 180</li><li>Cardamom bun: KES 150</li><li>Mandazi, six: KES 200</li><li>Banana bread slice: KES 220</li></ul>', 3, $loaf];
    $pages[] = ['bread', 'Bread', '<p>We bake six breads every day: country sourdough, rye and caraway, wholemeal tin, seeded spelt, white sandwich loaf and chapati-style flatbreads. Sourdough is fermented for eighteen hours.</p>', 4, $loaf];
    $pages[] = ['wholesale', 'Wholesale', '<p>We supply bread and pastries to cafés and offices in Kilimani, Kileleshwa and Westlands. Deliveries leave the bakery at 6am. Ask us for a wholesale price list.</p>', 6, 0];
    $pages[] = ['classes', 'Baking classes', '<p>Saturday morning classes for up to eight people: sourdough basics, laminated pastry, and cake decorating. Classes cost KES 4,500 and include lunch and everything you bake.</p>', 7, $team];
    $pages[] = ['faq', 'Questions', '<h2>Do you bake gluten-free?</h2><p>We bake a gluten-free banana loaf on Fridays, in a kitchen that also uses wheat.</p><h2>Can I change a cake order?</h2><p>Yes, up to two days before collection.</p><h2>Do you deliver?</h2><p>Within Nairobi, for KES 500.</p>', 8, 0];
    $pages[] = ['journal', 'Journal', '', 10, 0];

    $titles = [
        'How we choose our wheat', 'A morning at the ovens', 'Why our croissants take three days',
        'The cardamom bun, explained', 'Five cakes for a first birthday', 'Rye bread for beginners',
        'What a wedding cake tasting looks like', 'Passion fruit season is here', 'Meet Otieno, our head baker',
        'How to store sourdough', 'Our new spelt loaf', 'Mandazi on Sunday mornings',
        'Baking at altitude in Nairobi', 'Decorating with fresh flowers safely', 'The story of Mama Kilele',
        'A cake for two hundred guests', 'Cutting waste: bread pudding Fridays', 'Our butter comes from Limuru',
        'Easter hot cross buns', 'Christmas cake orders open', 'Feeding a starter on holiday',
        'Why we close on Mondays', 'A class of first-time bakers', 'Chocolate fudge cake, our bestseller',
        'Carrot cake with Kenyan walnuts', 'Lemon tart season', 'Making chapati-style flatbread',
        'How long does a cake keep?', 'Our wholesale van', 'Packaging without plastic',
        'A visit to the mill in Nakuru', 'Seeded loaves and where the seeds come from', 'Achieng\'s cake studio',
        'Sugar flowers, step by step', 'Banana bread with overripe bananas', 'Rainy-season proving times',
        'Bread for school lunches', 'Our bakery playlist', 'Thank you for nine years',
    ];
    $openings = [
        'Every morning starts before dawn at Kilele Bakery.',
        'Customers ask us about this more than anything else.',
        'This one began as an experiment in the cake studio.',
        'Wanjiru has been working on this for most of the year.',
        'It is one of the small things that make a loaf taste of Nairobi.',
    ];
    $middles = [
        'We weigh everything, keep notes on every batch, and change one thing at a time.',
        'The dough rests overnight in the cool room, and we shape it by hand at five.',
        'We buy from the same farmers each week, so we know how their flour behaves.',
        'Small batches mean we can adjust when the weather turns humid.',
        'We tasted twelve versions before we were happy to put it on the counter.',
    ];
    $closings = [
        'Come and try it at the counter, Tuesday to Sunday.',
        'Ask any of us about it when you visit; we love to talk about bread.',
        'You can order it for collection with three days\' notice.',
        'It sells out most days by four, so come early.',
        'Tell us what you think. We read every message.',
    ];
    $cats = ['News', 'Bread', 'Cakes', 'Pastries', 'Behind the counter'];
    foreach ($titles as $i => $title) {
        $slug = sanitize_title($title);
        $body = '<p>' . $openings[$i % 5] . ' ' . $middles[($i * 2) % 5] . '</p>'
            . '<p>' . $middles[($i * 3 + 1) % 5] . ' ' . $closings[$i % 5] . '</p>';
        $day = 1 + ($i % 28);
        $month = 1 + intdiv($i, 4) % 10;
        $date = sprintf('2026-%02d-%02d 07:%02d:00', $month, $day, ($i * 7) % 60);
        $thumb = [$loaf, $cake, 0, $team][$i % 4];
        $posts[] = [$slug, $title, $body, $date, $cats[$i % 5], $thumb];
    }
}

$ids = [];
foreach ($pages as [$slug, $title, $content, $order, $thumb]) {
    $ids[$slug] = kl_page($slug, $title, $content, $order, $thumb);
}
foreach ($posts as [$slug, $title, $content, $date, $category, $thumb]) {
    kl_post($slug, $title, $content, $date, $category, $thumb);
}

update_option('show_on_front', 'page');
update_option('page_on_front', $ids['home']);
if (isset($ids['journal'])) {
    update_option('page_for_posts', $ids['journal']);
}
wp_delete_term((int) get_option('default_category'), 'category'); // "Uncategorized", if empty

// The main menu, in page order.
$menu = wp_create_nav_menu('Main');
foreach ($ids as $slug => $id) {
    wp_update_nav_menu_item($menu, 0, [
        'menu-item-object-id' => $id,
        'menu-item-object' => 'page',
        'menu-item-type' => 'post_type',
        'menu-item-status' => 'publish',
    ]);
}
set_theme_mod('nav_menu_locations', ['main' => $menu]);

$count = count($pages) + count($posts);
WP_CLI::success("Kilele Bakery seeded ({$size}): " . count($pages) . " pages, " . count($posts) . " posts, {$count} in all.");
