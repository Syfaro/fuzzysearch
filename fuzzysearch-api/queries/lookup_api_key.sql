SELECT
    api_key.id,
    api_key.user_id,
    api_key.name_limit,
    api_key.image_limit,
    api_key.hash_limit,
    api_key.name
FROM
    api_key
WHERE
    api_key.key = $1
