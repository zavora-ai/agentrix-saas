<?php
/**
 * Kilele theme setup. Zavora's fixture (fictional bakery).
 */
add_action('after_setup_theme', function () {
    add_theme_support('title-tag');
    add_theme_support('post-thumbnails');
    add_theme_support('html5', ['search-form', 'gallery', 'caption', 'style', 'script']);
    register_nav_menus(['main' => 'Main menu']);
});

add_action('wp_enqueue_scripts', function () {
    $theme = wp_get_theme();
    wp_enqueue_style('kilele', get_stylesheet_uri(), [], $theme->get('Version'));
    $built = get_stylesheet_directory() . '/build/theme.css';
    if (is_readable($built)) {
        wp_enqueue_style('kilele-tokens', get_stylesheet_directory_uri() . '/build/theme.css', ['kilele'], filemtime($built));
    }
});
