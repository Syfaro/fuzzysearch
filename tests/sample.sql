-- Clear any existing data

TRUNCATE TABLE
    tag_to_post, submission, tag, artist,
    e621,
    tweet_media, tweet, twitter_user,
    rate_limit, api_key, account
    CASCADE;

-- Account and test API keys

INSERT INTO account (id, email, password) VALUES
    (1, 'test@example.com', 0);
INSERT INTO api_key (id, user_id, name, key, name_limit, image_limit, hash_limit) VALUES
    (1, 1, 'Test', 'test', 120, 120, 15);

-- FurAffinity sample data

INSERT INTO artist (id, name) VALUES
    (1, 'casual-dhole'),
    (2, 'kosseart'),
    (3, 'oce'),
    (4, 'psychonautic');

INSERT INTO tag (id, name) VALUES
    (1, 'syfaro'),
    (2, 'male'),
    (3, 'fox'),
    (4, 'purple'),
    (5, 'blue'),
    (6, 'light'),
    (7, 'dark'),
    (8, 'enjoying'),
    (9, 'delicious'),
    (10, 'grapes');

INSERT INTO submission (id, artist_id, hash, hash_int, url, filename, rating, posted_at, description, file_id, file_size, imported, removed, updated_at) VALUES
    (21060541, 1, '\x52eda52bc535b1a4', 5975613888001323428, 'https://d.facdn.net/art/casual-dhole/1473103034/1473103034.casual-dhole_fylninsyf2__web_.png', '1473103034.casual-dhole_fylninsyf2__web_.png', 'g', '2016-09-05 21:17:00+00', '', 1473103034, null, false, false, null),
    (33088558, 2, '\xb63326dd92c46ad8', -5317864001902449960, 'https://d.facdn.net/art/kosseart/1568810406/1568810406.kosseart_experimental-syfaro-fa.png', '1568810406.kosseart_experimental-syfaro-fa.png', 'g', '2019-09-18 13:40:00+00', '', 1568810406, null, false, false, null),
    (20449854, 3, '\x544565d3e5aad6ad', 6072371633344665261, 'https://d.facdn.net/art/oce/1467485464/1467485464.oce_syfaro-sketch-web.jpg', '1467485464.oce_syfaro-sketch-web.jpg', 'g', '2016-07-02 20:51:00+00', '', 1467485464, null, true, false, null),
    (19620670, 4, '\x5494a456b9b9ad92', 6094676888129219986, 'https://d.facdn.net/art/psychonautic/1460136557/1460136557.psychonautic_syfarore.png', '1460136557.psychonautic_syfarore.png', 'g', '2016-04-08 19:29:00+00', '', '1460136557', null, true, false, null);

INSERT INTO tag_to_post (tag_id, post_id) VALUES
    (1, 20449854),
    (2, 20449854),
    (3, 20449854),
    (4, 20449854),
    (5, 20449854),
    (6, 20449854),
    (7, 20449854),
    (8, 20449854),
    (9, 20449854),
    (10, 20449854);

-- e621 sample data

INSERT INTO e621 (id, hash, data, sha256) VALUES
    (934261, 6072371633344665261, '{"id": 934261, "file": {"ext": "jpg", "md5": "273210894ab3d9f02f02742acead73a2", "url": "https://static1.e621.net/data/27/32/273210894ab3d9f02f02742acead73a2.jpg", "size": 266900, "width": 681, "height": 900}, "tags": {"lore": [], "meta": [], "artist": ["amara_telgemeier"], "general": ["2016", "anthro", "biped", "eating", "food", "fruit", "fur", "grape", "looking_at_viewer", "male", "nude", "plant", "purple_body", "purple_fur", "purple_theme", "simple_background", "sitting", "solo"], "invalid": [], "species": ["canid", "canine", "fox", "mammal"], "character": ["syfaro"], "copyright": []}, "flags": {"deleted": false, "flagged": false, "pending": false, "note_locked": false, "rating_locked": false, "status_locked": false}, "pools": [], "score": {"up": 0, "down": 0, "total": 14}, "rating": "s", "sample": {"has": false, "url": "https://static1.e621.net/data/27/32/273210894ab3d9f02f02742acead73a2.jpg", "width": 681, "height": 900}, "preview": {"url": "https://static1.e621.net/data/preview/27/32/273210894ab3d9f02f02742acead73a2.jpg", "width": 113, "height": 150}, "sources": ["https://furrynetwork.com/artwork/1275945", "https://d3gz42uwgl1r1y.cloudfront.net/sy/syfaro/submission/2016/07/87f00959822f665716c58c4df43a27c2.jpg", "https://www.furaffinity.net/user/oce", "https://www.furaffinity.net/full/20449854/", "https://d.facdn.net/art/oce/1467485464/1467485464.oce_syfaro-sketch-web.jpg", "https://www.furaffinity.net/user/oce/"], "fav_count": 30, "change_seq": 26745767, "created_at": "2016-07-03T10:44:50.983-04:00", "updated_at": "2020-04-04T15:50:17.669-04:00", "approver_id": null, "description": "", "locked_tags": [], "uploader_id": 2083, "is_favorited": false, "comment_count": 0, "relationships": {"children": [], "parent_id": null, "has_children": false, "has_active_children": false}}', '\x26d16b09a372f780079af7b4bd13128ded8bf0f78395f40e2a3e307a3495955b');

--- Twitter sample data

INSERT INTO twitter_user (twitter_id, approved, data, last_update, max_id, completed_back, min_id) VALUES
    (1030062061856993282, true, '{"id": 1030062061856993282, "url": "https://t.co/9QXcrQ32Q2", "lang": null, "name": "𝕯𝖊𝖒𝖔𝖓 𝕯𝖔𝖌 𝕮𝖊𝖓𝖙𝖗𝖆𝖑™", "id_str": "1030062061856993282", "status": {"id": 1221685448407486465, "geo": null, "lang": "en", "text": "@folklaurel_ WHAT that''s so kind of you?? Thank you so much?!??? 😭💖", "place": null, "id_str": "1221685448407486465", "source": "<a href=\"https://mobile.twitter.com\" rel=\"nofollow\">Twitter Web App</a>", "entities": {"urls": [], "symbols": [], "hashtags": [], "user_mentions": [{"id": 2566142377, "name": "Colin 🐊 FC2020", "id_str": "2566142377", "indices": [0, 12], "screen_name": "folklaurel_"}]}, "favorited": false, "retweeted": false, "truncated": false, "created_at": "Mon Jan 27 06:44:43 +0000 2020", "coordinates": null, "contributors": null, "retweet_count": 0, "favorite_count": 0, "is_quote_status": false, "in_reply_to_user_id": 2566142377, "in_reply_to_status_id": 1221681145135300608, "in_reply_to_screen_name": "folklaurel_", "in_reply_to_user_id_str": "2566142377", "in_reply_to_status_id_str": "1221681145135300608"}, "entities": {"url": {"urls": [{"url": "https://t.co/9QXcrQ32Q2", "indices": [0, 23], "display_url": "deviantart.com/yodelinyote/ga…", "expanded_url": "https://www.deviantart.com/yodelinyote/gallery"}]}, "description": {"urls": []}}, "location": "St.Louis, Misery (MO)", "verified": false, "following": false, "protected": false, "time_zone": null, "created_at": "Thu Aug 16 12:01:47 +0000 2018", "utc_offset": null, "description": "Caim | 21 | transmasc - he/him ONLY! | I draw SFW furry art | Comms are CLOSED | Pfp: @kind7ed | Banner by me | Personal: @yappinyote", "followed_by": false, "geo_enabled": false, "screen_name": "yodelinyote", "listed_count": 13, "can_media_tag": false, "friends_count": 726, "is_translator": false, "notifications": false, "statuses_count": 4094, "default_profile": false, "followers_count": 5073, "translator_type": "none", "favourites_count": 10462, "profile_image_url": "http://pbs.twimg.com/profile_images/1160593354205405184/p-I8E7aX_normal.jpg", "profile_banner_url": "https://pbs.twimg.com/profile_banners/1030062061856993282/1571030121", "profile_link_color": "E81C4F", "profile_text_color": "000000", "follow_request_sent": false, "contributors_enabled": false, "has_extended_profile": true, "default_profile_image": false, "is_translation_enabled": false, "profile_background_tile": false, "profile_image_url_https": "https://pbs.twimg.com/profile_images/1160593354205405184/p-I8E7aX_normal.jpg", "profile_background_color": "000000", "profile_sidebar_fill_color": "000000", "profile_background_image_url": "http://abs.twimg.com/images/themes/theme1/bg.png", "profile_sidebar_border_color": "000000", "profile_use_background_image": false, "profile_background_image_url_https": "https://abs.twimg.com/images/themes/theme1/bg.png"}', '2021-02-21 00:49:04.59449', 1363167747291643906, false, 1218752964501868546);

INSERT INTO tweet (id, twitter_user_id, data) VALUES
    (1325965206934212608, 1030062061856993282, '{"id": 1325965206934212608, "geo": null, "lang": "en", "user": {"id": 1030062061856993282, "url": null, "lang": null, "name": "Caim", "id_str": "1030062061856993282", "entities": {"description": {"urls": [{"url": "https://t.co/mIG5vnu0lj", "indices": [82, 105], "display_url": "infurnalyote.carrd.co", "expanded_url": "http://infurnalyote.carrd.co"}]}}, "location": "Osage & Kickapoo land", "verified": false, "following": true, "protected": false, "time_zone": null, "created_at": "Thu Aug 16 12:01:47 +0000 2018", "utc_offset": null, "description": "♦️ Caim - 22 - transmasc (he/him) - BLM - ACAB ♦️\n♦️  Artist - Comms CLOSED ♦️\n♦️ https://t.co/mIG5vnu0lj - Pfp: @S0LARDOG ♦️", "geo_enabled": false, "screen_name": "infurnalyote", "listed_count": 58, "friends_count": 1108, "is_translator": false, "notifications": false, "statuses_count": 10721, "default_profile": false, "followers_count": 8793, "translator_type": "none", "favourites_count": 32013, "profile_image_url": "http://pbs.twimg.com/profile_images/1267191835728007173/oKM3jNzN_normal.jpg", "profile_banner_url": "https://pbs.twimg.com/profile_banners/1030062061856993282/1571030121", "profile_link_color": "E81C4F", "profile_text_color": "000000", "follow_request_sent": false, "contributors_enabled": false, "has_extended_profile": true, "default_profile_image": false, "is_translation_enabled": false, "profile_background_tile": false, "profile_image_url_https": "https://pbs.twimg.com/profile_images/1267191835728007173/oKM3jNzN_normal.jpg", "profile_background_color": "000000", "profile_sidebar_fill_color": "000000", "profile_background_image_url": "http://abs.twimg.com/images/themes/theme1/bg.png", "profile_sidebar_border_color": "000000", "profile_use_background_image": false, "profile_background_image_url_https": "https://abs.twimg.com/images/themes/theme1/bg.png"}, "place": null, "id_str": "1325965206934212608", "source": "<a href=\"https://mobile.twitter.com\" rel=\"nofollow\">Twitter Web App</a>", "entities": {"urls": [], "media": [{"id": 1325965104203042817, "url": "https://t.co/wL19uCgrAF", "type": "photo", "sizes": {"large": {"h": 1250, "w": 1000, "resize": "fit"}, "small": {"h": 680, "w": 544, "resize": "fit"}, "thumb": {"h": 150, "w": 150, "resize": "crop"}, "medium": {"h": 1200, "w": 960, "resize": "fit"}}, "id_str": "1325965104203042817", "indices": [20, 43], "media_url": "http://pbs.twimg.com/media/EmbGPKyWEAEi3JI.jpg", "display_url": "pic.twitter.com/wL19uCgrAF", "expanded_url": "https://twitter.com/infurnalyote/status/1325965206934212608/photo/1", "media_url_https": "https://pbs.twimg.com/media/EmbGPKyWEAEi3JI.jpg"}], "symbols": [], "hashtags": [], "user_mentions": []}, "favorited": false, "full_text": "Some more sillyness https://t.co/wL19uCgrAF", "retweeted": false, "truncated": false, "created_at": "Tue Nov 10 00:55:15 +0000 2020", "coordinates": null, "contributors": null, "retweet_count": 11, "favorite_count": 108, "is_quote_status": false, "extended_entities": {"media": [{"id": 1325965104203042817, "url": "https://t.co/wL19uCgrAF", "type": "photo", "sizes": {"large": {"h": 1250, "w": 1000, "resize": "fit"}, "small": {"h": 680, "w": 544, "resize": "fit"}, "thumb": {"h": 150, "w": 150, "resize": "crop"}, "medium": {"h": 1200, "w": 960, "resize": "fit"}}, "id_str": "1325965104203042817", "indices": [20, 43], "media_url": "http://pbs.twimg.com/media/EmbGPKyWEAEi3JI.jpg", "display_url": "pic.twitter.com/wL19uCgrAF", "expanded_url": "https://twitter.com/infurnalyote/status/1325965206934212608/photo/1", "media_url_https": "https://pbs.twimg.com/media/EmbGPKyWEAEi3JI.jpg"}, {"id": 1325965117285076993, "url": "https://t.co/wL19uCgrAF", "type": "photo", "sizes": {"large": {"h": 683, "w": 2048, "resize": "fit"}, "small": {"h": 227, "w": 680, "resize": "fit"}, "thumb": {"h": 150, "w": 150, "resize": "crop"}, "medium": {"h": 400, "w": 1200, "resize": "fit"}}, "id_str": "1325965117285076993", "indices": [20, 43], "media_url": "http://pbs.twimg.com/media/EmbGP7hWEAEysaF.jpg", "display_url": "pic.twitter.com/wL19uCgrAF", "expanded_url": "https://twitter.com/infurnalyote/status/1325965206934212608/photo/1", "media_url_https": "https://pbs.twimg.com/media/EmbGP7hWEAEysaF.jpg"}, {"id": 1325965183622246400, "url": "https://t.co/wL19uCgrAF", "type": "photo", "sizes": {"large": {"h": 500, "w": 1500, "resize": "fit"}, "small": {"h": 227, "w": 680, "resize": "fit"}, "thumb": {"h": 150, "w": 150, "resize": "crop"}, "medium": {"h": 400, "w": 1200, "resize": "fit"}}, "id_str": "1325965183622246400", "indices": [20, 43], "media_url": "http://pbs.twimg.com/media/EmbGTypW8AA65r_.jpg", "display_url": "pic.twitter.com/wL19uCgrAF", "expanded_url": "https://twitter.com/infurnalyote/status/1325965206934212608/photo/1", "media_url_https": "https://pbs.twimg.com/media/EmbGTypW8AA65r_.jpg"}]}, "display_text_range": [0, 19], "possibly_sensitive": false, "in_reply_to_user_id": 1030062061856993282, "in_reply_to_status_id": 1325964607509438470, "in_reply_to_screen_name": "infurnalyote", "in_reply_to_user_id_str": "1030062061856993282", "in_reply_to_status_id_str": "1325964607509438470"}');

INSERT INTO tweet_media (media_id, tweet_id, hash, url) VALUES
    (1325965183622246400, 1325965206934212608, -3140163608635666133, 'https://pbs.twimg.com/media/EmbGTypW8AA65r_.jpg:large'),
    (1325965104203042817, 1325965206934212608, 2641824390885488310, 'https://pbs.twimg.com/media/EmbGPKyWEAEi3JI.jpg:large'),
    (1325965117285076993, 1325965206934212608, 5517556289826018726, 'https://pbs.twimg.com/media/EmbGP7hWEAEysaF.jpg:large');
