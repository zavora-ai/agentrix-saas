-- W10: each kept report gets a token, for the weekly fix's confirm link and the one-tap stop.
ALTER TABLE home_contacts ADD COLUMN token TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS home_contacts_token ON home_contacts (token);
