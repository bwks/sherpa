SELECT id, agent, requester, first_seq, last_seq, last_at_ms, requested_model, github, prompt,
    sent_at_ms, started_at_ms, finished_at_ms, state, model, result, questions, answers, runs,
    cost_usd, input, output, cache_creation, cache_read, num_turns, duration_ms, duration_api_ms
FROM task
WHERE id = ?1
