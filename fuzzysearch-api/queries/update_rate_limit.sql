INSERT INTO
    rate_limit (api_key_id, time_window, group_name, count)
VALUES
    ($1, $2, $3, $4) ON CONFLICT ON CONSTRAINT unique_window DO
UPDATE
set
    count = rate_limit.count + $4 RETURNING rate_limit.count
