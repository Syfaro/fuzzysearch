SELECT
    submission.id,
    submission.url,
    submission.filename,
    submission.file_id,
    submission.rating,
    submission.posted_at,
    submission.hash_int hash,
    artist.name artist
FROM
    submission
    LEFT JOIN artist ON artist.id = submission.artist_id
WHERE
    file_id = $1
LIMIT
    10;
