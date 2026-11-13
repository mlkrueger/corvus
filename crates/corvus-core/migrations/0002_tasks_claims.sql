-- 0002 tasks, dependencies and claims. See spec/system/02-data-model.md. Never edit once applied.

CREATE TABLE tasks (
  id              TEXT PRIMARY KEY,
  title           TEXT NOT NULL CHECK (trim(title) <> ''),
  body            TEXT NOT NULL DEFAULT '' CHECK (length(body) <= 20000),
  priority        INTEGER NOT NULL DEFAULT 0,
  status          TEXT NOT NULL DEFAULT 'open'
                  CHECK (status IN ('open', 'claimed', 'done', 'cancelled')),
  pool            TEXT NOT NULL DEFAULT 'default',
  recipient_agent TEXT REFERENCES agents (name),
  creator         TEXT NOT NULL REFERENCES agents (name),
  result_summary  TEXT,
  cancel_reason   TEXT,
  cancelled_by    TEXT REFERENCES agents (name),
  idempotency_key TEXT,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,
  completed_at    INTEGER,
  cancelled_at    INTEGER
) STRICT;

CREATE UNIQUE INDEX tasks_idempotency ON tasks (creator, idempotency_key)
  WHERE idempotency_key IS NOT NULL;
-- The claim scan: ORDER BY priority DESC, created_at, rowid. SQLite appends rowid (insertion order) to every
-- index implicitly and does not allow naming it, so the index lists the first two keys only.
CREATE INDEX tasks_open_scan ON tasks (priority DESC, created_at) WHERE status = 'open';
CREATE INDEX tasks_status ON tasks (status);

CREATE TABLE task_dependencies (
  task_id       TEXT NOT NULL REFERENCES tasks (id),
  depends_on_id TEXT NOT NULL REFERENCES tasks (id),
  PRIMARY KEY (task_id, depends_on_id),
  CHECK (task_id <> depends_on_id)
) STRICT;

CREATE INDEX task_dependencies_depends_on ON task_dependencies (depends_on_id);

CREATE TABLE claims (
  id               TEXT PRIMARY KEY,
  task_id          TEXT NOT NULL REFERENCES tasks (id),
  session_id       TEXT NOT NULL REFERENCES sessions (id),
  agent_name       TEXT NOT NULL REFERENCES agents (name),
  claimed_at       INTEGER NOT NULL,
  lease_expires_at INTEGER NOT NULL,
  released_at      INTEGER,
  end_reason       TEXT CHECK (end_reason IN ('released', 'completed', 'expired', 'cancelled')),
  CHECK ((released_at IS NULL) = (end_reason IS NULL))
) STRICT;

-- P2: at most one active claim per task and per session, even for a buggy service.
CREATE UNIQUE INDEX claims_active_task ON claims (task_id) WHERE released_at IS NULL;
CREATE UNIQUE INDEX claims_active_session ON claims (session_id) WHERE released_at IS NULL;
CREATE INDEX claims_active_lease ON claims (lease_expires_at) WHERE released_at IS NULL;
CREATE INDEX claims_task ON claims (task_id);
