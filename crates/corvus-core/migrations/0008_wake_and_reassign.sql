-- 0008 waking broadcasts and reassigned mail (D0024, D0025). See spec/system/02-data-model.md.
-- Never edit once applied.

-- A broadcast the operator marked to wake idle sessions.
ALTER TABLE messages ADD COLUMN wake INTEGER NOT NULL DEFAULT 0 CHECK (wake IN (0, 1));
-- For a reassigned copy: the original message.
ALTER TABLE messages ADD COLUMN reassigned_from INTEGER REFERENCES messages (id);

-- A receipt is an acknowledgement by the agent, or the operator moving the message to another agent.
ALTER TABLE receipts ADD COLUMN kind TEXT NOT NULL DEFAULT 'ack' CHECK (kind IN ('ack', 'reassigned'));
ALTER TABLE receipts ADD COLUMN reassigned_to TEXT REFERENCES agents (name)
  CHECK ((kind = 'reassigned') = (reassigned_to IS NOT NULL));
