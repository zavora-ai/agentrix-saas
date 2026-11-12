<!doctype html>
<html <?php language_attributes(); ?>>
<head>
<meta charset="<?php bloginfo('charset'); ?>">
<meta name="viewport" content="width=device-width, initial-scale=1">
<?php wp_head(); ?>
</head>
<body <?php body_class(); ?>>
<?php wp_body_open(); ?>
<header class="kl-header">
	<div class="kl-wrap">
		<a class="kl-brand" href="<?php echo esc_url(home_url('/')); ?>"><?php bloginfo('name'); ?></a>
		<p class="kl-tagline"><?php bloginfo('description'); ?></p>
		<nav class="kl-nav" aria-label="Main">
			<?php wp_nav_menu(['theme_location' => 'main', 'container' => false, 'fallback_cb' => false, 'depth' => 1]); ?>
		</nav>
	</div>
</header>
<main class="kl-main"><div class="kl-wrap">
