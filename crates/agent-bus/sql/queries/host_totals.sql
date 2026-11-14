-- How many responses the host has, what the priced ones cost, how many are not priced, and
-- the first and last.
SELECT COUNT(*), SUM(cost_usd), SUM(cost_usd IS NULL), MIN(at_ms), MAX(at_ms) FROM host_usage
