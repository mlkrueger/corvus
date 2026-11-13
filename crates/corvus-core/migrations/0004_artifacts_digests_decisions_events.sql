-- 0004 artifacts, digests, decisions and audit events. See spec/system/02-data-model.md.
-- Never edit once applied.

CREATE TABLE artifacts (
  id          TEXT PRIMARY KEY,
  kind        TEXT NOT NULL CHECK (kind IN ('file', 'branch', 'worktree', 'commit', 'pr')),
  path        TEXT NOT NULL CHECK (length(path) BETWEEN 1 AND 1024),
  commit_hash TEXT CHECK (length(commit_hash) BETWEEN 7 AND 40 AND commit_hash NOT GLOB '*[^0-9a-f]*'),
  description TEXT NOT NULL DEFAULT '',
  task_id     TEXT REFERENCES tasks (id),
  thread_id   TEXT REFERENCES threads (id),
  message_id  INTEGER REFERENCES messages (id),
  created_at  INTEGER NOT NULL,
  CHECK ((task_id IS NOT NULL) + (thread_id IS NOT NULL) + (message_id IS NOT NULL) = 1),
  CHECK (path NOT LIKE '/%' AND path <> '..' AND path NOT LIKE '../%'
     AND path NOT LIKE '%/../%' AND path NOT LIKE '%/..' AND instr(path, char(0)) = 0)
) STRICT;

CREATE INDEX artifacts_task ON artifacts (task_id) WHERE task_id IS NOT NULL;
CREATE INDEX artifacts_thread ON artifacts (thread_id) WHERE thread_id IS NOT NULL;
CREATE INDEX artifacts_message ON artifacts (message_id) WHERE message_id IS NOT NULL;

CREATE TABLE digests (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  scope           TEXT NOT NULL CHECK (scope IN ('global', 'task', 'thread', 'agent')),
  ref             TEXT,
  scope_key       TEXT GENERATED ALWAYS AS
                  (CASE WHEN scope = 'global' THEN 'global' ELSE scope || ':' || ref END) STORED,
  summary         TEXT NOT NULL CHECK (trim(summary) <> ''),
  author          TEXT NOT NULL REFERENCES agents (name),
  covers_through  INTEGER,
  idempotency_key TEXT,
  created_at      INTEGER NOT NULL,
  CHECK ((scope = 'global') = (ref IS NULL))
) STRICT;

CREATE UNIQUE INDEX digests_idempotency ON digests (author, idempotency_key)
  WHERE idempotency_key IS NOT NULL;
-- Latest per scope key = max(id).
CREATE INDEX digests_scope_key ON digests (scope_key, id DESC);

CREATE TABLE decisions (
  id              TEXT PRIMARY KEY,
  title           TEXT NOT NULL CHECK (trim(title) <> ''),
  decision        TEXT NOT NULL,
  rationale       TEXT NOT NULL DEFAULT '',
  decided_by      TEXT NOT NULL REFERENCES agents (name),
  task_id         TEXT REFERENCES tasks (id),
  thread_id       TEXT REFERENCES threads (id),
  supersedes      TEXT REFERENCES decisions (id),
  idempotency_key TEXT,
  created_at      INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX decisions_idempotency ON decisions (decided_by, idempotency_key)
  WHERE idempotency_key IS NOT NULL;
CREATE INDEX decisions_supersedes ON decisions (supersedes);

-- No foreign keys: writing an audit row must never fail because of what it references.
CREATE TABLE events (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  kind       TEXT NOT NULL,
  summary    TEXT NOT NULL,
  agent_name TEXT,
  session_id TEXT,
  task_id    TEXT,
  thread_id  TEXT,
  message_id INTEGER,
  created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX events_task ON events (task_id, id) WHERE task_id IS NOT NULL;
CREATE INDEX events_thread ON events (thread_id, id) WHERE thread_id IS NOT NULL;
CREATE INDEX events_agent ON events (agent_name, id) WHERE agent_name IS NOT NULL;
