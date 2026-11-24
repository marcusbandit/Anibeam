-- The episode fallback (#1461) ------------------------------------------------------------
-- A season ani.zip has not mapped yet gets its episode details from TVmaze, through the
-- TheTVDB id ani.zip gives another season of the same franchise, and a season with no clear
-- logo of its own borrows the nearest franchise season's. Keyed by the AniList id, like the
-- anizip_* tables. A fetch that answered replaces a show's episode rows; one that failed
-- leaves everything as it was.

CREATE TABLE fallback_shows (
    anilist_id   INTEGER PRIMARY KEY,
    via_anilist  INTEGER,                    -- the franchise season the TVmaze show was found through
    tvmaze_id    INTEGER,
    season       INTEGER,                    -- the TVmaze season this entry is
    logo_url     TEXT,                       -- the nearest franchise season's clear logo
    fetched_at   INTEGER NOT NULL
);

CREATE TABLE fallback_episodes (
    anilist_id   INTEGER NOT NULL REFERENCES fallback_shows(anilist_id) ON DELETE CASCADE,
    number       INTEGER NOT NULL,
    title        TEXT,                       -- NULL for a placeholder
    synopsis     TEXT,                       -- plain text
    still_url    TEXT,
    runtime      INTEGER,
    aired_at     INTEGER,                    -- Unix seconds
    PRIMARY KEY (anilist_id, number)
);
