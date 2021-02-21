CREATE TABLE e621 (
    id INTEGER PRIMARY KEY,
    hash BIGINT,
    data JSONB,
    sha256 BYTEA,
    hash_error TEXT
);

CREATE INDEX ON e621 (sha256);
