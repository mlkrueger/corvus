-- 0005 append-only enforcement (P4, D0010) and wake-up hints (D0016). Never edit once applied.

CREATE TRIGGER messages_no_update BEFORE UPDATE ON messages
BEGIN SELECT RAISE(ABORT, 'append-only table'); END;
CREATE TRIGGER messages_no_delete BEFORE DELETE ON messages
BEGIN SELECT RAISE(ABORT, 'append-only table'); END;

CREATE TRIGGER digests_no_update BEFORE UPDATE ON digests
BEGIN SELECT RAISE(ABORT, 'append-only table'); END;
CREATE TRIGGER digests_no_delete BEFORE DELETE ON digests
BEGIN SELECT RAISE(ABORT, 'append-only table'); END;

CREATE TRIGGER events_no_update BEFORE UPDATE ON events
BEGIN SELECT RAISE(ABORT, 'append-only table'); END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON events
BEGIN SELECT RAISE(ABORT, 'append-only table'); END;

-- Plumbing, not history: written only by the triggers below, pruned to the newest 1,000 rows.
CREATE TABLE hints (
  id    INTEGER PRIMARY KEY AUTOINCREMENT,
  topic TEXT NOT NULL
) STRICT;

CREATE TRIGGER messages_hint AFTER INSERT ON messages
BEGIN INSERT INTO hints (topic) VALUES ('inbox:' || coalesce(NEW.recipient, '*')); END;

CREATE TRIGGER tasks_hint_insert AFTER INSERT ON tasks
BEGIN INSERT INTO hints (topic) VALUES ('tasks'); END;
CREATE TRIGGER tasks_hint_update AFTER UPDATE ON tasks
BEGIN INSERT INTO hints (topic) VALUES ('tasks'); END;
