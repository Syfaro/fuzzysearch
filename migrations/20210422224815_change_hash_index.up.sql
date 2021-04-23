DROP INDEX bk_furaffinity_hash;
DROP INDEX bk_e621_hash;
DROP INDEX bk_twitter_hash;
DROP INDEX bk_weasyl_hash;

CREATE INDEX submission_hash_int_idx ON submission (hash_int);
CREATE INDEX e621_hash_idx ON e621 (hash);
CREATE INDEX tweet_media_hash_idx ON tweet_media (hash);
CREATE INDEX weasyl_hash_idx ON weasyl (hash);
