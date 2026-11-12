-- W1 · the Chief WebAgent's answers, step by step, for the console's Activity view: each
-- model call, tool read, dial check and subagent the Chief asked, with times. Writer:
-- agentrix-kernel only. The owner's own question is kept; visitors' words never are (tools pass
-- desks' passed questions as data, and the trace keeps only tool names and counts).
CREATE TABLE webagent_traces (
    trace_id   TEXT PRIMARY KEY,
    site_id    TEXT NOT NULL,
    surface    TEXT NOT NULL,          -- chat, voice, console
    question   TEXT NOT NULL,
    answer     TEXT NOT NULL,
    steps      TEXT NOT NULL,          -- JSON list of Step
    sources    TEXT NOT NULL,          -- JSON list of Source
    created_at TEXT NOT NULL
) STRICT;
CREATE INDEX webagent_traces_site ON webagent_traces (site_id, created_at);
