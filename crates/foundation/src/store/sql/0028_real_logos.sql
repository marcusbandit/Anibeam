-- Real logos (#1463) ------------------------------------------------------------------------
-- ani.zip files TheTVDB's square icons under Clearlogo as well as its clear logo art, and the
-- series head drew an icon where the title belongs. Only /clearlogo/ art is a logo: the rest
-- goes, from ani.zip's rows and from the logos the fallback borrowed. The match ignores case,
-- as the parser's does. Every walk is due again, so a logo borrowed under the old
-- nearest-first order is picked again with the previous season first.
DELETE FROM anizip_images WHERE kind = 'Clearlogo' AND instr(lower(url), '/clearlogo/') = 0;
UPDATE fallback_shows SET logo_url = NULL WHERE logo_url IS NOT NULL AND instr(lower(logo_url), '/clearlogo/') = 0;
UPDATE fallback_shows SET fetched_at = 0;

-- The person's per-show choice between the logo and the title in text: 1 the logo, 0 the
-- text, NULL to follow the app's setting.
ALTER TABLE series ADD COLUMN logo_choice INTEGER;
