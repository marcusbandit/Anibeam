-- Logo languages, fetched as picked (#1475 review) ------------------------------------------
-- tvdb_logo_wanted sorted the logos into English, Japanese and the rest by TheTVDB's language
-- alone, while the series page picks by TheTVDB's language, else ani.zip's for the same url,
-- cut to its base (`en-US` is `en`). A logo TheTVDB named no language for then fell in with
-- the rest, so the Japanese one the page would pick was never fetched. The view now sorts by
-- that same coalesced base language, so it fetches what the pick can choose.

DROP VIEW tvdb_logo_wanted;

CREATE VIEW tvdb_logo_wanted AS
WITH named AS (
    SELECT k.anilist_id, l.url, l.rank,
           replace(lower(trim(coalesce(l.lang, (
               SELECT a.lang FROM anizip_images a
               WHERE a.anilist_id = k.anilist_id AND a.kind = 'Clearlogo' AND a.url = l.url
           )))), '_', '-') AS lang
    FROM tvdb_logo_asks k
    JOIN anizip_shows z ON z.anilist_id = k.anilist_id AND z.tvdb_id = k.tvdb_id
    JOIN tvdb_logos l ON l.tvdb_id = k.tvdb_id
    WHERE k.anilist_id IN (SELECT anilist_id FROM series WHERE provider = 'anilist')
      AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = l.url AND v.verdict = 0)
),
based AS (
    SELECT anilist_id, url, rank,
           CASE WHEN instr(lang, '-') > 0 THEN substr(lang, 1, instr(lang, '-') - 1) ELSE lang END AS base
    FROM named
)
SELECT anilist_id, url FROM (
    SELECT anilist_id, url, row_number() OVER (
        PARTITION BY anilist_id, CASE WHEN base = 'en' THEN 0 WHEN base = 'ja' THEN 1 ELSE 2 END
        ORDER BY rank
    ) AS pick
    FROM based
) WHERE pick = 1;
