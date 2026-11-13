-- 0001 agents and sessions. See spec/system/02-data-model.md. Never edit once applied.

CREATE TABLE agents (
  name       TEXT PRIMARY KEY
             CHECK (length(name) BETWEEN 1 AND 64
                AND name NOT GLOB '*[^A-Za-z0-9_.-]*'
                AND substr(name, 1, 1) GLOB '[A-Za-z0-9]'
                -- length() and GLOB stop at a NUL; without this, text after one would be unchecked.
                AND instr(name, char(0)) = 0),
  created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE sessions (
  id              TEXT PRIMARY KEY,
  agent_name      TEXT NOT NULL REFERENCES agents (name),
  client_name     TEXT NOT NULL,
  metadata        TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),
  name_generated  INTEGER NOT NULL DEFAULT 0 CHECK (name_generated IN (0, 1)),
  started_at      INTEGER NOT NULL,
  last_seen_at    INTEGER NOT NULL,
  ended_at        INTEGER,
  end_reason      TEXT CHECK (end_reason IN ('ended', 'auto_ended')),
  idempotency_key TEXT,
  CHECK ((ended_at IS NULL) = (end_reason IS NULL))
) STRICT;

CREATE UNIQUE INDEX sessions_idempotency ON sessions (agent_name, idempotency_key)
  WHERE idempotency_key IS NOT NULL;
CREATE INDEX sessions_live_last_seen ON sessions (last_seen_at) WHERE ended_at IS NULL;
CREATE INDEX sessions_agent ON sessions (agent_name);
