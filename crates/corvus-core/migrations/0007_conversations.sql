-- 0007 conversation links: which agent a tool conversation is (D0023). See spec/system/02-data-model.md.
-- Never edit once applied.

CREATE TABLE conversations (
  tool            TEXT NOT NULL
                  CHECK (length(tool) BETWEEN 1 AND 32 AND tool NOT GLOB '*[^a-z0-9-]*'),
  conversation_id TEXT NOT NULL
                  CHECK (length(conversation_id) BETWEEN 1 AND 128
                     AND conversation_id NOT GLOB '*[^A-Za-z0-9_.-]*'),
  agent_name      TEXT NOT NULL REFERENCES agents (name),
  directory       TEXT,
  carried_from    TEXT,
  created_at      INTEGER NOT NULL,
  PRIMARY KEY (tool, conversation_id)
) STRICT;

CREATE INDEX conversations_agent ON conversations (agent_name, created_at);

-- A conversation belongs to one agent for good.
CREATE TRIGGER conversations_no_update BEFORE UPDATE ON conversations
BEGIN SELECT RAISE(ABORT, 'conversation links are immutable'); END;
CREATE TRIGGER conversations_no_delete BEFORE DELETE ON conversations
BEGIN SELECT RAISE(ABORT, 'conversation links are immutable'); END;
