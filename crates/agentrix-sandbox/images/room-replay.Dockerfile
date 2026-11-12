# room-replay (S2-05; design §3.1; R4.1-R4.3): replays an old WordPress site in a sealed room,
# from its database dump (with its wp-content, so its own plugins and infections run) or a WXR
# export, and records what it serves. The contract is in room-replay/replay.sh.
#
# Built on the sandbox VM (design D8); its digest goes in images/digests.toml:
#   docker build -f crates/agentrix-sandbox/images/room-replay.Dockerfile \
#     -t room-replay:dev crates/agentrix-sandbox/images/room-replay
#
# The runner starts it with gVisor, a read-only root, user 65534, a tmpfs /tmp (noexec) and the
# job's egress proxy (empty allowlist) as its only way out.
FROM php:8.3-cli-bookworm

ARG WP_VERSION=6.8.2
ARG WPCLI_VERSION=2.12.0

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
      mariadb-server mariadb-client jq curl ca-certificates unzip; \
    rm -rf /var/lib/apt/lists/* /var/lib/mysql /run/mysqld; \
    docker-php-ext-install -j"$(nproc)" mysqli; \
    curl -fsSLo /tmp/wp.tar.gz "https://wordpress.org/wordpress-${WP_VERSION}.tar.gz"; \
    echo "$(curl -fsSL "https://wordpress.org/wordpress-${WP_VERSION}.tar.gz.sha1")  /tmp/wp.tar.gz" | sha1sum -c -; \
    tar -xzf /tmp/wp.tar.gz -C /opt; \
    rm /tmp/wp.tar.gz; \
    curl -fsSLo /usr/local/bin/wp "https://github.com/wp-cli/wp-cli/releases/download/v${WPCLI_VERSION}/wp-cli-${WPCLI_VERSION}.phar"; \
    echo "$(curl -fsSL "https://github.com/wp-cli/wp-cli/releases/download/v${WPCLI_VERSION}/wp-cli-${WPCLI_VERSION}.phar.sha512")  /usr/local/bin/wp" | sha512sum -c -; \
    chmod 0755 /usr/local/bin/wp; \
    curl -fsSLo /tmp/importer.zip https://downloads.wordpress.org/plugin/wordpress-importer.latest-stable.zip; \
    mkdir -p /opt/replay-plugins; \
    unzip -q /tmp/importer.zip -d /opt/replay-plugins; \
    rm /tmp/importer.zip

COPY replay.sh router.php prepend.php /opt/replay/
COPY php.ini /usr/local/etc/php/conf.d/zz-replay.ini
RUN chmod 0755 /opt/replay/replay.sh && chmod 0644 /opt/replay/router.php /opt/replay/prepend.php

USER 65534:65534
ENV HOME=/tmp
ENTRYPOINT ["/opt/replay/replay.sh"]
