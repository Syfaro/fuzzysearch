CREATE TABLE webhook (
    id SERIAL PRIMARY KEY,
    account_id INTEGER REFERENCES account (id),
    endpoint TEXT NOT NULL
);
