-- agentrix-records (W2): who proved itself at the agent door, on the order or hold it raised
-- (additive). v3, "Verify at sample doors": the verified id, who vouched for it, and how
-- (`web-bot-auth` or `token`). Rows from before, and orders not raised by a proven agent, keep NULL.
ALTER TABLE rec_orders ADD COLUMN agent_id TEXT;
ALTER TABLE rec_orders ADD COLUMN agent_vouched_by TEXT;
ALTER TABLE rec_orders ADD COLUMN agent_proof_method TEXT;
