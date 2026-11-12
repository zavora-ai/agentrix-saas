-- Registry columns on Brick 1's sites and budgets (W3; specs/platform/03-data.md §2.4, rows 8 and
-- 9 of W8's data-fixes.md). Range 0300–0399. Additive only: nullable columns, so the old binary
-- runs on this schema. Nothing writes them yet except the origin backfill below.

ALTER TABLE sites ADD COLUMN org_id TEXT;
ALTER TABLE sites ADD COLUMN plan TEXT;
ALTER TABLE sites ADD COLUMN cell TEXT;
ALTER TABLE sites ADD COLUMN residency TEXT;
ALTER TABLE sites ADD COLUMN archetype TEXT;
ALTER TABLE sites ADD COLUMN tier TEXT;
ALTER TABLE sites ADD COLUMN currency TEXT;
ALTER TABLE sites ADD COLUMN languages TEXT;
ALTER TABLE sites ADD COLUMN lifecycle TEXT;
-- An IANA zone such as Africa/Nairobi, so quiet hours follow DST where a site has it.
ALTER TABLE sites ADD COLUMN time_zone TEXT;
-- fixture (Zavora's own), sample (a -sample host) or real.
ALTER TABLE sites ADD COLUMN origin TEXT CHECK (origin IS NULL OR origin IN ('fixture', 'sample', 'real'));

UPDATE sites SET origin = CASE
    WHEN fixture = 1 THEN 'fixture'
    WHEN instr(domain, '.') > 0 AND substr(domain, 1, instr(domain, '.') - 1) LIKE '%-sample' THEN 'sample'
    WHEN instr(domain, '.') = 0 AND domain LIKE '%-sample' THEN 'sample'
    ELSE 'real'
END
WHERE origin IS NULL;

-- Budgets per WebAgent and mission, and the currency the owner reads them in. Amounts stay
-- micro-USD; display converts at the site's posted rate.
ALTER TABLE budgets ADD COLUMN scope TEXT;
ALTER TABLE budgets ADD COLUMN display_currency TEXT;
