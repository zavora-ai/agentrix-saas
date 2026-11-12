-- W9 · Messages: the consent words behind a message, kept with its outbox row (W10's website
-- check and weekly fix). Additive and nullable, so an older binary still runs on this schema.
ALTER TABLE message_outbox ADD COLUMN consent TEXT;
