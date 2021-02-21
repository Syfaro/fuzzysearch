CREATE TABLE twitter_user (
    twitter_id BIGINT PRIMARY KEY,
    approved BOOLEAN NOT NULL DEFAULT false,
    data JSONB,
    last_update TIMESTAMP WITHOUT TIME ZONE,
    max_id BIGINT,
    completed_back BOOLEAN NOT NULL DEFAULT false,
    min_id BIGINT
);

CREATE INDEX ON twitter_user (last_update);
CREATE INDEX ON twitter_user (lower(data->>'screen_name'));
CREATE INDEX ON twitter_user (min_id);
CREATE INDEX ON twitter_user (twitter_id, approved);
CREATE INDEX ON twitter_user (((data->'protected')::boolean));

CREATE TABLE tweet (
    id BIGINT PRIMARY KEY,
    twitter_user_id BIGINT NOT NULL REFERENCES twitter_user (twitter_id),
    data JSONB
);

CREATE TABLE tweet_media (
    media_id BIGINT NOT NULL,
    tweet_id BIGINT NOT NULL REFERENCES tweet (id),
    hash BIGINT,
    url TEXT,

    PRIMARY KEY (media_id, tweet_id)
);
