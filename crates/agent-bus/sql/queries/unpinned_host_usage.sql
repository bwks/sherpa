-- The responses attribution may move: those not pinned by `work reassign`.
SELECT message_id, session, agent_id, at_ms, work FROM host_usage WHERE pinned = 0
