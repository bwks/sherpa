-- Price the response ?1, unless it is priced: a priced response is never priced again.
UPDATE host_usage SET cost_usd = ?2 WHERE message_id = ?1 AND cost_usd IS NULL
