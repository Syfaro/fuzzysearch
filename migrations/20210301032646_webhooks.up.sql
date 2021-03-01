CREATE TABLE webhook (
    id SERIAL PRIMARY KEY,
    account_id INTEGER REFERENCES account (id),
    endpoint TEXT NOT NULL,
    filtered BOOLEAN NOT NULL DEFAULT false
);

CREATE TABLE webhook_site (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

INSERT INTO webhook_site (id, name) VALUES (1, 'FurAffinity');

CREATE TABLE webhook_filter (
    webhook_id INTEGER NOT NULL REFERENCES webhook (id),
    site_id INTEGER NOT NULL REFERENCES webhook_site (id),
    artist_name TEXT NOT NULL,

    PRIMARY KEY (webhook_id, site_id, artist_name)
);
