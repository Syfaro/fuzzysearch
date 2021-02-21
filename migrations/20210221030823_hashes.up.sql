CREATE TABLE hashes (
    id SERIAL PRIMARY KEY,
    hash BIGINT NOT NULL,
    furaffinity_id INTEGER UNIQUE REFERENCES submission (id),
    e621_id INTEGER UNIQUE REFERENCES e621 (id),
    twitter_id BIGINT REFERENCES tweet (id)
);

CREATE FUNCTION hashes_insert_furaffinity()
    RETURNS trigger
    LANGUAGE plpgsql
AS $$
BEGIN
    if NEW.hash_int IS NOT NULL THEN
        INSERT INTO hashes (furaffinity_id, hash) VALUES (NEW.id, NEW.hash_int);
    END IF;

    RETURN NEW;
END;
$$;

CREATE FUNCTION hashes_insert_e621()
    RETURNS trigger
    LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.hash IS NOT NULL THEN
        IF exists(SELECT 1 FROM hashes WHERE hashes.e621_id = NEW.id) THEN
            UPDATE hashes SET hashes.hash = NEW.hash WHERE e621_id = NEW.id;
        ELSE
            INSERT INTO hashes (e621_id, hash) VALUES (NEW.id, NEW.hash);
        END IF;
    END IF;

    RETURN NEW;
END;
$$;

CREATE FUNCTION hashes_insert_twitter()
    RETURNS trigger
    LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.hash IS NOT NULL THEN
        INSERT INTO hashes (twitter_id, hash) VALUES (NEW.tweet_id, NEW.hash);
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER hashes_insert_furaffinity AFTER INSERT ON submission
    FOR EACH ROW EXECUTE PROCEDURE hashes_insert_furaffinity();
CREATE TRIGGER hashes_insert_e621 AFTER INSERT ON e621
    FOR EACH ROW EXECUTE PROCEDURE hashes_insert_e621();
CREATE TRIGGER hashes_insert_twitter AFTER INSERT ON tweet_media
    FOR EACH ROW EXECUTE PROCEDURE hashes_insert_twitter();

INSERT INTO hashes (furaffinity_id, hash)
    SELECT id, hash_int FROM submission WHERE hash_int IS NOT NULL
    ON CONFLICT DO NOTHING;
INSERT INTO hashes (e621_id, hash)
    SELECT id, hash FROM e621 WHERE hash IS NOT NULL
    ON CONFLICT DO NOTHING;
INSERT INTO hashes (twitter_id, hash)
    SELECT tweet_id, hash FROM tweet_media WHERE hash IS NOT NULL
    ON CONFLICT DO NOTHING;

CREATE INDEX ON hashes USING spgist (hash bktree_ops);

CREATE FUNCTION hashes_notify_inserted()
    RETURNS trigger
    LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM pg_notify('fuzzysearch_hash_added'::text,
        json_build_object('id', NEW.id, 'hash', NEW.hash)::text);
    RETURN NEW;
END;
$$;

CREATE TRIGGER hashes_notify_inserted AFTER INSERT ON hashes
    FOR EACH ROW EXECUTE PROCEDURE hashes_notify_inserted();
