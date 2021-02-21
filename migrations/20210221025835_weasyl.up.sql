CREATE TABLE weasyl (
    id INTEGER PRIMARY KEY,
    hash BIGINT,
    data JSONB,
    sha256 BYTEA,
    file_size INTEGER
);

CREATE INDEX ON weasyl (sha256);
