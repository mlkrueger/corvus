-- 0006 work-source tables shared by the Jira and Linear sources. See spec/system/07-work-sources.md.
-- Never edit once applied.

-- The operator-editable filter; structured fields only; never credentials.
CREATE TABLE source_settings (
  source     TEXT PRIMARY KEY CHECK (source IN ('jira', 'linear')),
  filter     TEXT NOT NULL CHECK (json_valid(filter)),
  updated_at INTEGER NOT NULL
) STRICT;

-- The board snapshot; replaced wholesale each pass.
CREATE TABLE source_tickets (
  source          TEXT NOT NULL CHECK (source IN ('jira', 'linear')),
  key             TEXT NOT NULL,
  project         TEXT,
  summary         TEXT,
  status          TEXT,
  status_category TEXT,
  priority        INTEGER,
  url             TEXT,
  updated_stamp   TEXT,
  resolved_at     INTEGER,
  assignee        TEXT,
  PRIMARY KEY (source, key)
) STRICT;

CREATE TABLE source_links (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  source        TEXT NOT NULL CHECK (source IN ('jira', 'linear')),
  ticket_key    TEXT NOT NULL,
  task_id       TEXT NOT NULL UNIQUE REFERENCES tasks (id),
  updated_stamp TEXT NOT NULL,                        -- compared for equality only, never parsed
  comment_hwm   INTEGER NOT NULL DEFAULT 0,           -- highest processed Comment.order_key
  hwm_ids       TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(hwm_ids)),
  in_scope      INTEGER NOT NULL DEFAULT 1 CHECK (in_scope IN (0, 1)),
  manual        INTEGER NOT NULL DEFAULT 0 CHECK (manual IN (0, 1)),
  created_at    INTEGER NOT NULL
) STRICT;

-- The newest link per ticket is the live link.
CREATE INDEX source_links_ticket ON source_links (source, ticket_key, id DESC);

-- last_error is a sanitised category + message; never a URL with credentials or a header.
CREATE TABLE source_sync_state (
  source       TEXT PRIMARY KEY CHECK (source IN ('jira', 'linear')),
  last_sync_at INTEGER,
  last_stats   TEXT CHECK (last_stats IS NULL OR json_valid(last_stats)),
  last_error   TEXT
) STRICT;
