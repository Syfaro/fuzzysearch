DROP TABLE hashes;
DROP FUNCTION hashes_notify_inserted CASCADE;
DROP FUNCTION hashes_insert_furaffinity CASCADE;
DROP FUNCTION hashes_insert_e621 CASCADE;
DROP FUNCTION hashes_insert_twitter CASCADE;

CREATE FUNCTION update_notify_furaffinity()
    RETURNS trigger
    LANGUAGE plpgsql
AS $$
BEGIN
    if NEW.hash_int IS NOT NULL THEN
        PERFORM pg_notify('fuzzysearch_hash_added'::text,
            json_build_object('hash', NEW.hash_int)::text);
        RETURN NEW;
    END IF;

    RETURN NEW;
END;
$$;

CREATE FUNCTION update_notify_others()
    RETURNS trigger
    LANGUAGE plpgsql
AS $$
BEGIN
    if NEW.hash IS NOT NULL THEN
        PERFORM pg_notify('fuzzysearch_hash_added'::text,
            json_build_object('hash', NEW.hash)::text);
        RETURN NEW;
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER update_notify_furaffinity AFTER INSERT OR UPDATE ON submission
    FOR EACH ROW EXECUTE PROCEDURE update_notify_furaffinity();
CREATE TRIGGER update_notify_e621 AFTER INSERT OR UPDATE ON e621
    FOR EACH ROW EXECUTE PROCEDURE update_notify_others();
CREATE TRIGGER update_notify_twitter AFTER INSERT OR UPDATE ON tweet_media
    FOR EACH ROW EXECUTE PROCEDURE update_notify_others();
CREATE TRIGGER update_notify_weasyl AFTER INSERT OR UPDATE ON weasyl
    FOR EACH ROW EXECUTE PROCEDURE update_notify_others();
