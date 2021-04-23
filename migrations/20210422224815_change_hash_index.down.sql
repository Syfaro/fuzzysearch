DROP INDEX submission_hash_int_idx;
DROP INDEX e621_hash_idx;
DROP INDEX tweet_media_hash_idx;
DROP INDEX weasyl_hash_idx;

CREATE INDEX bk_furaffinity_hash ON submission USING spgist (hash_int bktree_ops);
CREATE INDEX bk_e621_hash ON e621 USING spgist (hash bktree_ops);
CREATE INDEX bk_twitter_hash ON tweet_media USING spgist (hash bktree_ops);
CREATE INDEX bk_weasyl_hash ON weasyl USING spgist (hash bktree_ops);
