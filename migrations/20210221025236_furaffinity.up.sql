CREATE TABLE artist (
    id SERIAL PRIMARY KEY,
    name TEXT UNIQUE NOT NULL
);

CREATE TABLE submission (
    id INTEGER PRIMARY KEY,
    artist_id INTEGER REFERENCES artist (id),
    hash BYTEA,
    hash_int BIGINT,
    url TEXT,
    filename TEXT,
    rating CHAR(1),
    posted_at TIMESTAMP WITH TIME ZONE,
    description TEXT,
    file_id INTEGER,
    file_size INTEGER,
    file_sha256 BYTEA,
    imported BOOLEAN DEFAULT false,
    removed BOOLEAN,
    updated_at TIMESTAMP WITH TIME ZONE
);

CREATE INDEX ON submission (file_id);
CREATE INDEX ON submission (imported);
CREATE INDEX ON submission (posted_at);
CREATE INDEX ON submission (artist_id);
CREATE INDEX ON submission (file_sha256) WHERE file_sha256 IS NOT NULL;
CREATE INDEX ON submission (lower(url));
CREATE INDEX ON submission (lower(filename));

CREATE TABLE tag (
    id SERIAL PRIMARY KEY,
    name TEXT UNIQUE NOT NULL
);

CREATE TABLE tag_to_post (
    tag_id INTEGER NOT NULL REFERENCES tag (id),
    post_id INTEGER NOT NULL REFERENCES submission (id),

    PRIMARY KEY (tag_id, post_id)
);
