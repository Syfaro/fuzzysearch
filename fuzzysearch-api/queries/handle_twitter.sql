SELECT
    exists(
        SELECT
            1
        FROM
            twitter_user
        WHERE
            lower(data ->> 'screen_name') = lower($1)
    ) "exists!";
