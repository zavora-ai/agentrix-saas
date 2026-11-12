-- Visit counts without trackers (docs/06-website-experience.md §4.9, docs/09 §5.3). Owner: W8
-- (agentrix-store::visits). Daily totals per site, in the site's own day: the route (never the
-- URL), the referring site's host, a device class, the country when the edge knows it, and the
-- release seq. No cookie, identifier, IP address or user agent is kept. Exact counts stay here;
-- every screen and export shows a cell under 5 as "fewer than 5".
CREATE TABLE visit_counts (
    site_id       TEXT NOT NULL REFERENCES sites (site_id),
    day           TEXT NOT NULL CHECK (length(day) = 10),
    route         TEXT NOT NULL CHECK (length(route) BETWEEN 1 AND 200),
    referrer_host TEXT NOT NULL DEFAULT '',
    device        TEXT NOT NULL CHECK (device IN ('phone', 'tablet', 'desktop')),
    country       TEXT NOT NULL DEFAULT '',
    release_seq   INTEGER NOT NULL DEFAULT 0 CHECK (release_seq >= 0),
    visits        INTEGER NOT NULL CHECK (visits >= 1),
    PRIMARY KEY (site_id, day, route, referrer_host, device, country, release_seq)
) STRICT;
