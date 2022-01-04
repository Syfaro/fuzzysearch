SELECT
    api_key.id,
    api_key.name_limit,
    api_key.image_limit,
    api_key.hash_limit,
    api_key.name,
    account.email owner_email
FROM
    api_key
    JOIN account ON account.id = api_key.user_id
WHERE
    api_key.key = $1
