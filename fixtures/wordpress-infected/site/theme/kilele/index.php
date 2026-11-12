<?php get_header(); ?>
<?php if (have_posts()) : while (have_posts()) : the_post(); ?>
	<article id="post-<?php the_ID(); ?>" <?php post_class('kl-entry'); ?>>
		<?php if (is_singular()) : ?>
			<?php if (!is_front_page()) : ?><h1><?php the_title(); ?></h1><?php endif; ?>
		<?php else : ?>
			<h2><a href="<?php the_permalink(); ?>"><?php the_title(); ?></a></h2>
		<?php endif; ?>
		<?php if (get_post_type() === 'post') : ?>
			<p class="kl-meta"><?php echo esc_html(get_the_date()); ?></p>
		<?php endif; ?>
		<?php if (is_singular()) { the_content(); } else { the_excerpt(); } ?>
	</article>
<?php endwhile; ?>
	<?php the_posts_pagination(); ?>
<?php else : ?>
	<p>Nothing here yet.</p>
<?php endif; ?>
<?php get_footer(); ?>
