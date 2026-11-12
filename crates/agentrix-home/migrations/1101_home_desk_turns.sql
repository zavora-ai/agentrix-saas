-- W10: how many questions agentrix.website's desks answered, and whether a model wrote the
-- answer, for the team's states on screen 2. Never the question or the answer.
CREATE TABLE IF NOT EXISTS home_desk_turns (
  desk TEXT NOT NULL,
  ai INTEGER NOT NULL DEFAULT 0,
  at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS home_desk_turns_at ON home_desk_turns (at);
