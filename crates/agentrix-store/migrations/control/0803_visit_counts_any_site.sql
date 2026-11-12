-- Visit counts for every site, including the sample sites, which live in agentrix-samples' own
-- store rather than in control.db's `sites` (W8). 0800 tied visit_counts to `sites` with a
-- foreign key, so a sample site's visit would be refused. The table is rebuilt with the same
-- columns, in the same order, without that key; the site id is checked in code by the host
-- lookup that resolves it. Binaries built before this keep working on the result.
CREATE TABLE visit_counts_any (
    site_id       TEXT NOT NULL CHECK (length(site_id) BETWEEN 1 AND 80),
    day           TEXT NOT NULL CHECK (length(day) = 10),
    route         TEXT NOT NULL CHECK (length(route) BETWEEN 1 AND 200),
    referrer_host TEXT NOT NULL DEFAULT '',
    device        TEXT NOT NULL CHECK (device IN ('phone', 'tablet', 'desktop')),
    country       TEXT NOT NULL DEFAULT '',
    release_seq   INTEGER NOT NULL DEFAULT 0 CHECK (release_seq >= 0),
    visits        INTEGER NOT NULL CHECK (visits >= 1),
    PRIMARY KEY (site_id, day, route, referrer_host, device, country, release_seq)
) STRICT;

INSERT INTO visit_counts_any SELECT site_id, day, route, referrer_host, device, country,
    release_seq, visits FROM visit_counts;

-- Every row copied, or the migration fails and nothing changes.
CREATE TEMP TABLE w8_visits_copied (ok INTEGER NOT NULL CHECK (ok = 1));
INSERT INTO w8_visits_copied
SELECT (SELECT COUNT(*) FROM visit_counts_any) = (SELECT COUNT(*) FROM visit_counts)
   AND (SELECT COALESCE(SUM(visits), 0) FROM visit_counts_any)
     = (SELECT COALESCE(SUM(visits), 0) FROM visit_counts);
DROP TABLE w8_visits_copied;

DROP TABLE visit_counts;
ALTER TABLE visit_counts_any RENAME TO visit_counts;
