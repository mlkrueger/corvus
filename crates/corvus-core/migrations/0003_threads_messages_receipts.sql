-- 0003 threads, messages and receipts. See spec/system/02-data-model.md. Never edit once applied.

CREATE TABLE threads (
  id         TEXT PRIMARY KEY,
  title      TEXT NOT NULL CHECK (trim(title) <> ''),
  task_id    TEXT UNIQUE REFERENCES tasks (id),
  status     TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
  creator    TEXT NOT NULL REFERENCES agents (name),
  key        TEXT UNIQUE,
  created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE messages (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  thread_id       TEXT NOT NULL REFERENCES threads (id),
  sender          TEXT NOT NULL REFERENCES agents (name),
  recipient       TEXT REFERENCES agents (name),
  type            TEXT NOT NULL
                  CHECK (type IN ('note', 'question', 'answer', 'handoff', 'status', 'decision')),
  in_reply_to     INTEGER,
  body            TEXT NOT NULL CHECK (trim(body) <> '' AND length(body) <= 20000),
  session_id      TEXT REFERENCES sessions (id),
  idempotency_key TEXT,
  created_at      INTEGER NOT NULL,
  UNIQUE (id, thread_id),
  -- A reply stays in its thread.
  FOREIGN KEY (in_reply_to, thread_id) REFERENCES messages (id, thread_id)
) STRICT;

CREATE UNIQUE INDEX messages_idempotency ON messages (sender, idempotency_key)
  WHERE idempotency_key IS NOT NULL;
CREATE INDEX messages_recipient ON messages (recipient, id);
CREATE INDEX messages_thread ON messages (thread_id, id);
CREATE INDEX messages_answers ON messages (in_reply_to) WHERE type = 'answer';

CREATE TABLE receipts (
  message_id INTEGER NOT NULL REFERENCES messages (id),
  agent_name TEXT    NOT NULL REFERENCES agents (name),
  session_id TEXT    REFERENCES sessions (id),
  acked_at   INTEGER NOT NULL,
  PRIMARY KEY (message_id, agent_name)
) STRICT;
