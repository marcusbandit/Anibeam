-- ani.zip (#1036) ---------------------------------------------------------------------
-- ani.zip maps an AniList id to AniDB and TheTVDB and answers with both sides' episode
-- list and the show's artwork in one request. Keyed by the AniList id, the way
-- anilist_media is, so a rematch never reads another show's rows. A fetch that answered
-- replaces a show's rows whole; one that failed leaves them as they were.

CREATE TABLE anizip_shows (
    anilist_id  INTEGER PRIMARY KEY,
    found       INTEGER NOT NULL,            -- 0 when ani.zip has no entry for the id
    mal_id      INTEGER,                     -- the mapping's MAL id
    anidb_id    INTEGER,
    tvdb_id     INTEGER,
    fetched_at  INTEGER NOT NULL
);

CREATE TABLE anizip_episodes (
    anilist_id  INTEGER NOT NULL REFERENCES anizip_shows(anilist_id) ON DELETE CASCADE,
    key         TEXT NOT NULL,               -- ani.zip's own: '1', 'S2' a special, 'C1' a credit
    number      INTEGER,                     -- the entry's episode number; NULL for every other key
    season      INTEGER,                     -- TheTVDB's season, 0 for specials
    episode     INTEGER,                     -- TheTVDB's number within that season
    absolute    INTEGER,                     -- TheTVDB's absolute number
    title       TEXT,                        -- English, else romaji; NULL for a placeholder
    synopsis    TEXT,
    still_url   TEXT,
    runtime     INTEGER,                     -- minutes
    air_date    TEXT,                        -- YYYY-MM-DD
    aired_at    INTEGER,                     -- Unix seconds, when the reply gave a time
    PRIMARY KEY (anilist_id, key)
);
CREATE INDEX anizip_episodes_number ON anizip_episodes(anilist_id, number);

CREATE TABLE anizip_images (
    anilist_id  INTEGER NOT NULL REFERENCES anizip_shows(anilist_id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,               -- 'Clearlogo' | 'Fanart' | 'Banner' | 'Poster'
    url         TEXT NOT NULL,
    lang        TEXT,
    rank        INTEGER NOT NULL,            -- the reply's own order
    PRIMARY KEY (anilist_id, kind, url)
);
