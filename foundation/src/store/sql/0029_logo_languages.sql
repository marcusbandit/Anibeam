-- Logo languages (#1470) --------------------------------------------------------------------
-- ani.zip's artwork does not say reliably which language a clear logo is in, and TheTVDB's own
-- pages do. tvdb_logos holds a TheTVDB series' clear logos with the language each one's
-- artwork page names, so two seasons of one series share them. tvdb_logo_asks says when an
-- AniList entry last asked, and which of ani.zip's logos it had then: a set that changed is
-- asked again, the same set is not until a long while has passed.

CREATE TABLE tvdb_logos (
    tvdb_id  INTEGER NOT NULL,
    url      TEXT NOT NULL,
    lang     TEXT,                       -- 'en', 'ja', another code or name; NULL when unnamed
    rank     INTEGER NOT NULL,           -- the series page's own order
    PRIMARY KEY (tvdb_id, url)
);

CREATE TABLE tvdb_logo_asks (
    anilist_id  INTEGER PRIMARY KEY,
    tvdb_id     INTEGER NOT NULL,
    logo_set    TEXT NOT NULL,           -- ani.zip's Clearlogo urls when it asked, sorted, ';' joined
    fetched_at  INTEGER NOT NULL
);
CREATE INDEX tvdb_logo_asks_tvdb ON tvdb_logo_asks(tvdb_id);

-- The TheTVDB logos a series in the library may draw: for each entry, the first in English,
-- the first in Japanese and the first in anything else, so whichever the logo language
-- setting picks is on disk. Only while ani.zip still maps the entry to that series.
CREATE VIEW tvdb_logo_wanted AS
SELECT anilist_id, url FROM (
    SELECT k.anilist_id, l.url, row_number() OVER (
        PARTITION BY k.anilist_id,
            CASE WHEN l.lang = 'en' THEN 0 WHEN l.lang = 'ja' THEN 1 ELSE 2 END
        ORDER BY l.rank
    ) AS pick
    FROM tvdb_logo_asks k
    JOIN anizip_shows z ON z.anilist_id = k.anilist_id AND z.tvdb_id = k.tvdb_id
    JOIN tvdb_logos l ON l.tvdb_id = k.tvdb_id
    WHERE k.anilist_id IN (SELECT anilist_id FROM series WHERE provider = 'anilist')
) WHERE pick = 1;
