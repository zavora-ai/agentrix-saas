-- The owner console's own tables (W5, range 0500–0599). Writer: agentrix-shell (ShellStore).
-- Run by agentrix_shell::migrate with its own table, _sqlx_migrations_agentrix_shell. Additive only.

-- Quiet hours per site, in the site's local time ("22:00", "06:00"). No row: 22:00 to 06:00.
CREATE TABLE IF NOT EXISTS shell_quiet_hours (
    site_id    TEXT PRIMARY KEY,
    start_hm   TEXT NOT NULL,
    end_hm     TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    updated_by TEXT NOT NULL
);

-- Each Freeze of a site (07 §5.5). A row with no unfrozen_at is in force. Freezing needs only the
-- owner's session; unfreezing is an approval over `unfreeze_effect` (an EffectRequest, as JSON),
-- whose receipt is `unfreeze_receipt`.
CREATE TABLE IF NOT EXISTS shell_freezes (
    id                TEXT PRIMARY KEY,
    site_id           TEXT NOT NULL,
    frozen_at         TEXT NOT NULL,
    frozen_by         TEXT NOT NULL,
    how               TEXT NOT NULL,
    unfreeze_effect   TEXT,
    unfrozen_at       TEXT,
    unfrozen_by       TEXT,
    unfreeze_method   TEXT,
    unfreeze_receipt  TEXT
);
CREATE INDEX IF NOT EXISTS shell_freezes_site ON shell_freezes (site_id, frozen_at);

-- When each owner last opened their site's console, for "while you were away".
CREATE TABLE IF NOT EXISTS shell_visits (
    account      TEXT NOT NULL,
    site_id      TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (account, site_id)
);
