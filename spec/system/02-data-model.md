# 02 — Data model

SQLite schema for the domain in brief §3. DDL here is a **sketch that fixes names and constraints**; the
migrations are authoritative once written. Storage choice: [D0013](../decisions/0013-storage-sqlite.md).

## Conventions

- Every invariant that can be a constraint **is** a constraint. Application checks give good errors; the store
  gives guarantees.
- All tables are `STRICT`. `foreign_keys = ON` on every connection (set by the one opener,
  [01](01-architecture.md#database-access)).
- Types: ids `TEXT` (UUIDv7) or `INTEGER PRIMARY KEY AUTOINCREMENT`; timestamps `INTEGER` UTC milliseconds,
  supplied by the application clock ([D0007](../decisions/0007-time-source.md)); booleans `INTEGER` with
  `CHECK (x IN (0,1))`; JSON `TEXT` with `CHECK (json_valid(x))`; enumerations `TEXT` with `CHECK (… IN (…))`.
- SQLite has no regex in constraints; character-class rules use `GLOB`
  (for example `x NOT GLOB '*[^A-Za-z0-9_.-]*'` = "only these characters").
- Nothing is ever deleted (except `hints` pruning and the tracker snapshot replace).

## Tables

### agents
```sql
agents (
  name       TEXT PRIMARY KEY
             CHECK (length(name) BETWEEN 1 AND 64
                AND name NOT GLOB '*[^A-Za-z0-9_.-]*'
                AND substr(name, 1, 1) GLOB '[A-Za-z0-9]'
                AND instr(name, char(0)) = 0),       -- length() and GLOB stop at a NUL
  created_at INTEGER NOT NULL
)
```
Created on first use with `INSERT … ON CONFLICT DO NOTHING`. Generated names are reserved by the same insert:
a row returned means the name is ours (brief §3 Session).

### sessions
```sql
sessions (
  id              TEXT PRIMARY KEY,
  agent_name      TEXT NOT NULL REFERENCES agents,
  client_name     TEXT NOT NULL,
  metadata        TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),
  name_generated  INTEGER NOT NULL DEFAULT 0 CHECK (name_generated IN (0,1)),
  started_at      INTEGER NOT NULL,
  last_seen_at    INTEGER NOT NULL,
  ended_at        INTEGER,
  end_reason      TEXT CHECK (end_reason IN ('ended','auto_ended')),
  idempotency_key TEXT
)
UNIQUE (agent_name, idempotency_key) WHERE idempotency_key IS NOT NULL
INDEX (last_seen_at) WHERE ended_at IS NULL
```

### conversations
```sql
conversations (
  tool            TEXT NOT NULL                       -- 'claude-code' | 'opencode' | 'codex' (enum in corvus-domain)
                  CHECK (length(tool) BETWEEN 1 AND 32 AND tool NOT GLOB '*[^a-z0-9-]*'),
  conversation_id TEXT NOT NULL                       -- the tool's own id for the conversation
                  CHECK (length(conversation_id) BETWEEN 1 AND 128
                     AND conversation_id NOT GLOB '*[^A-Za-z0-9_.-]*'),
  agent_name      TEXT NOT NULL REFERENCES agents,
  directory       TEXT,                               -- working directory when linked, if known
  carried_from    TEXT,                               -- the previous conversation_id, when created by a clear
  created_at      INTEGER NOT NULL,
  PRIMARY KEY (tool, conversation_id)
)
INDEX (agent_name, created_at)
```
Which agent a tool conversation is ([D0023](../decisions/0023-agent-identity-follows-conversation.md)). Rows are
never updated or deleted: a conversation belongs to one agent for good; an agent can have many conversations
(resumes share one row; each clear adds one). Written by `register_session` and by the carry-over
([03](03-services.md#sessions-and-presence)).

### tasks
```sql
tasks (
  id              TEXT PRIMARY KEY,
  title           TEXT NOT NULL CHECK (trim(title) <> ''),
  body            TEXT NOT NULL DEFAULT '' CHECK (length(body) <= 20000),
  priority        INTEGER NOT NULL DEFAULT 0,
  status          TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','claimed','done','cancelled')),
  pool            TEXT NOT NULL DEFAULT 'default',
  recipient_agent TEXT REFERENCES agents,
  creator         TEXT NOT NULL REFERENCES agents,
  result_summary  TEXT,
  cancel_reason   TEXT,
  cancelled_by    TEXT REFERENCES agents,
  idempotency_key TEXT,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,
  completed_at    INTEGER,
  cancelled_at    INTEGER
)
UNIQUE (creator, idempotency_key) WHERE idempotency_key IS NOT NULL
INDEX (priority DESC, created_at) WHERE status = 'open'   -- the claim scan; SQLite appends rowid (= insertion
                                                           -- order) to every index implicitly and cannot name it
```

### task_dependencies
```sql
task_dependencies (
  task_id       TEXT NOT NULL REFERENCES tasks,
  depends_on_id TEXT NOT NULL REFERENCES tasks,
  PRIMARY KEY (task_id, depends_on_id),
  CHECK (task_id <> depends_on_id)
)
INDEX (depends_on_id)
```
Dependencies are set only at creation and can only point at existing tasks, so cycles cannot form.

### claims
```sql
claims (
  id               TEXT PRIMARY KEY,
  task_id          TEXT NOT NULL REFERENCES tasks,
  session_id       TEXT NOT NULL REFERENCES sessions,
  agent_name       TEXT NOT NULL REFERENCES agents,
  claimed_at       INTEGER NOT NULL,
  lease_expires_at INTEGER NOT NULL,
  released_at      INTEGER,
  end_reason       TEXT CHECK (end_reason IN ('released','completed','expired','cancelled')),
  CHECK ((released_at IS NULL) = (end_reason IS NULL))
)
UNIQUE (task_id)    WHERE released_at IS NULL     -- at most one active claim per task      (P2)
UNIQUE (session_id) WHERE released_at IS NULL     -- at most one active claim per session   (P2)
INDEX (lease_expires_at) WHERE released_at IS NULL
```
The two partial unique indexes are the last line of defence for P2: even a buggy service cannot double-claim.

### threads
```sql
threads (
  id         TEXT PRIMARY KEY,
  title      TEXT NOT NULL CHECK (trim(title) <> ''),
  task_id    TEXT UNIQUE REFERENCES tasks,          -- one thread per task
  status     TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','closed')),
  creator    TEXT NOT NULL REFERENCES agents,
  key        TEXT UNIQUE,                           -- idempotent standalone threads
  created_at INTEGER NOT NULL
)
```
Direct threads use the reserved key `direct:<a>:<b>` with the two agent names sorted. `:` cannot appear in an
agent name, so the key is unambiguous. `create_thread` rejects caller keys starting with `direct:`.

### messages (append-only)
```sql
messages (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  thread_id       TEXT NOT NULL REFERENCES threads,
  sender          TEXT NOT NULL REFERENCES agents,
  recipient       TEXT REFERENCES agents,           -- NULL = broadcast
  type            TEXT NOT NULL CHECK (type IN ('note','question','answer','handoff','status','decision')),
  in_reply_to     INTEGER,
  body            TEXT NOT NULL CHECK (trim(body) <> '' AND length(body) <= 20000),
  session_id      TEXT REFERENCES sessions,
  idempotency_key TEXT,
  created_at      INTEGER NOT NULL,
  wake            INTEGER NOT NULL DEFAULT 0 CHECK (wake IN (0,1)),   -- a broadcast that wakes idle sessions
  reassigned_from INTEGER REFERENCES messages,                        -- the original, for a reassigned copy
  UNIQUE (id, thread_id),
  FOREIGN KEY (in_reply_to, thread_id) REFERENCES messages (id, thread_id)   -- reply stays in its thread
)
UNIQUE (sender, idempotency_key) WHERE idempotency_key IS NOT NULL
INDEX (recipient, id); INDEX (thread_id, id); INDEX (in_reply_to) WHERE type = 'answer'
```

### receipts
```sql
receipts (
  message_id INTEGER NOT NULL REFERENCES messages,
  agent_name TEXT    NOT NULL REFERENCES agents,
  session_id TEXT    REFERENCES sessions,
  acked_at   INTEGER NOT NULL,
  kind          TEXT NOT NULL DEFAULT 'ack' CHECK (kind IN ('ack','reassigned')),
  reassigned_to TEXT REFERENCES agents,
  CHECK ((kind = 'reassigned') = (reassigned_to IS NOT NULL)),
  PRIMARY KEY (message_id, agent_name)
)
```
Read state lives here, so `messages` rows never change. A `reassigned` receipt is the operator moving the
message to another agent, not the agent acknowledging it
([D0025](../decisions/0025-operator-reassigns-stranded-mail.md)).

**Unread for agent A** (the one definition, used everywhere):
```sql
(m.recipient = :a OR m.recipient IS NULL) AND m.sender <> :a
AND NOT EXISTS (SELECT 1 FROM receipts r WHERE r.message_id = m.id AND r.agent_name = :a)
```
**Unresolved question**: `m.type = 'question' AND NOT EXISTS (answer a WHERE a.type='answer' AND a.in_reply_to = m.id)`.

### artifacts
```sql
artifacts (
  id          TEXT PRIMARY KEY,
  kind        TEXT NOT NULL CHECK (kind IN ('file','branch','worktree','commit','pr')),
  path        TEXT NOT NULL,                        -- fully validated in domain
  commit_hash TEXT CHECK (length(commit_hash) BETWEEN 7 AND 40 AND commit_hash NOT GLOB '*[^0-9a-f]*'),
  description TEXT NOT NULL DEFAULT '',
  task_id     TEXT REFERENCES tasks,
  thread_id   TEXT REFERENCES threads,
  message_id  INTEGER REFERENCES messages,
  created_at  INTEGER NOT NULL,
  CHECK ((task_id IS NOT NULL) + (thread_id IS NOT NULL) + (message_id IS NOT NULL) = 1),
  CHECK (path NOT LIKE '/%' AND path <> '..' AND path NOT LIKE '../%'
     AND path NOT LIKE '%/../%' AND path NOT LIKE '%/..' AND instr(path, char(0)) = 0)
)
```

### digests (append-only)
```sql
digests (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  scope           TEXT NOT NULL CHECK (scope IN ('global','task','thread','agent')),
  ref             TEXT,                             -- task id | thread id | agent name; NULL for global
  scope_key       TEXT GENERATED ALWAYS AS (CASE WHEN scope = 'global' THEN 'global' ELSE scope || ':' || ref END) STORED,
  summary         TEXT NOT NULL CHECK (trim(summary) <> ''),
  author          TEXT NOT NULL REFERENCES agents,
  covers_through  INTEGER,                          -- message id
  idempotency_key TEXT,
  created_at      INTEGER NOT NULL,
  CHECK ((scope = 'global') = (ref IS NULL))
)
UNIQUE (author, idempotency_key) WHERE idempotency_key IS NOT NULL
INDEX (scope_key, id DESC)                          -- latest per scope key = max(id)
```
The service verifies `ref` resolves to an existing task / thread / agent (not-found otherwise).

### decisions
```sql
decisions (
  id              TEXT PRIMARY KEY,
  title           TEXT NOT NULL CHECK (trim(title) <> ''),
  decision        TEXT NOT NULL,
  rationale       TEXT NOT NULL DEFAULT '',
  decided_by      TEXT NOT NULL REFERENCES agents,
  task_id         TEXT REFERENCES tasks,
  thread_id       TEXT REFERENCES threads,
  supersedes      TEXT REFERENCES decisions,
  idempotency_key TEXT,
  created_at      INTEGER NOT NULL
)
UNIQUE (decided_by, idempotency_key) WHERE idempotency_key IS NOT NULL
INDEX (supersedes)
```
Superseded = some other row has `supersedes = id`; computed at read time, so rows never change.

### events (append-only)
```sql
events (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  kind       TEXT NOT NULL,         -- e.g. 'task.created', 'task.claimed', 'lease.expired', 'session.auto_ended'
  summary    TEXT NOT NULL,         -- never contains a message body
  agent_name TEXT, session_id TEXT, task_id TEXT, thread_id TEXT, message_id INTEGER,
  created_at INTEGER NOT NULL
)
```
No foreign keys: writing an audit row must never fail because of what it references. The full list of `kind`
values is an enum in `corvus-domain`.

### hints
```sql
hints ( id INTEGER PRIMARY KEY AUTOINCREMENT, topic TEXT NOT NULL )   -- 'inbox:<agent>' | 'inbox:*' | 'tasks'
```
Wake-up plumbing ([D0016](../decisions/0016-wakeups-hints-table.md)). Written only by triggers; pruned to the
newest 1,000 rows. Not history, not exported, not append-only.

### Work-source tables
See [07](07-work-sources.md#storage): `source_settings`, `source_tickets`, `source_links`, `source_sync_state`
(shared by the Jira and Linear sources).

## Triggers

| Trigger | On | Purpose |
|---|---|---|
| `<table>_no_update` `BEFORE UPDATE`, `<table>_no_delete` `BEFORE DELETE` → `SELECT RAISE(ABORT, 'append-only table')` | `messages`, `digests`, `events` | P4. [D0010](../decisions/0010-append-only-enforcement.md) |
| `conversations_no_update` `BEFORE UPDATE`, `conversations_no_delete` `BEFORE DELETE` → `SELECT RAISE(ABORT, 'conversation links are immutable')` | `conversations` | A conversation is one agent for good. [D0023](../decisions/0023-agent-identity-follows-conversation.md) |
| `messages_hint` `AFTER INSERT` | `messages` | `INSERT INTO hints(topic) VALUES ('inbox:' || coalesce(NEW.recipient, '*'))` |
| `tasks_hint_insert` `AFTER INSERT`, `tasks_hint_update` `AFTER UPDATE` | `tasks` | `INSERT INTO hints(topic) VALUES ('tasks')` |

- SQLite has no `TRUNCATE`; a bare `DELETE FROM t` fires the delete trigger for every row (the truncate
  optimisation is disabled on tables with delete triggers), so it is rejected too.
- Hints are written by triggers so no code path can add a message or change a task without one (brief §4.3),
  and they commit atomically with the change.

## Migrations

In-house migrator in `corvus-core::db` ([D0014](../decisions/0014-rust-stack-rusqlite.md)).

- Files `crates/corvus-core/migrations/NNNN_name.sql`, embedded in the binary at compile time.
- Bookkeeping: `schema_migrations (version INTEGER PRIMARY KEY, name TEXT, checksum TEXT, applied_at INTEGER)`
  plus `PRAGMA user_version` = highest applied version (the cheap skew check).
- A run is **one `BEGIN IMMEDIATE` transaction** — that is the lock (P10). Inside it: verify every applied
  row's SHA-256 checksum against the embedded file (mismatch → hard error, nothing applied); apply pending
  migrations in numeric order; record them; set `user_version`; commit. Concurrent runners queue on the write
  lock and then find nothing to do.
- One-way: there are no down migrations. An applied migration is never edited (the checksum check enforces it).
- **When it runs**: automatically when a writing process opens the database (`corvus mcp`, CLI commands,
  `serve`, `watch`, the desktop app), and explicitly with `corvus migrate`. The hook never migrates; on a
  mismatch it exits silently.
- **Newer database than binary** → refuse to run, with a message to upgrade Corvus.
- **Backup first**: before migrating a non-empty database, write `corvus.db.pre-v<N>.bak` with `VACUUM INTO`.
- `health` reports the highest applied version.
- Planned order: `0001_agents_sessions`, `0002_tasks_claims`, `0003_threads_messages_receipts`,
  `0004_artifacts_digests_decisions_events`, `0005_append_only_and_hints`, `0006_work_sources`.
