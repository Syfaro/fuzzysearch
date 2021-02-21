CREATE TABLE account (
    id SERIAL PRIMARY KEY,
    email TEXT UNIQUE NOT NULL,
    password TEXT NOT NULL,
    email_verifier TEXT
);

CREATE TABLE api_key (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES account (id),
    name TEXT,
    key TEXT UNIQUE NOT NULL,
    name_limit SMALLINT NOT NULL,
    image_limit SMALLINT NOT NULL,
    hash_limit SMALLINT NOT NULL
);

CREATE TABLE rate_limit (
    api_key_id INTEGER NOT NULL REFERENCES api_key (id),
    time_window BIGINT NOT NULL,
    group_name TEXT NOT NULL,
    count SMALLINT NOT NULL DEFAULT 0,

    CONSTRAINT unique_window
        PRIMARY KEY (api_key_id, time_window, group_name)
);
