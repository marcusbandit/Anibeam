//! Reading which torrent each library file came from (`file_origins`, #241).
//! The downloads team records them; the series page and On disk read them.
//!
//! An origin is current only while it is at least as new as the file: a
//! file written after its origin was recorded (downloaded over by another
//! release) has none until the torrent client is read again. The writer
//! (`anibeam_downloads::origins::record_from`) treats such a row as unknown,
//! and every reader goes through here so none of them trusts it either.

use std::collections::HashMap;

use rusqlite::{Connection, params_from_iter};

use crate::store::sql::placeholders;
use anibeam_contract::CoreError;

/// One recorded origin: the torrent's info hash, lower case, and its name.
#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    pub hash: String,
    pub name: String,
}

/// The current origins of these paths, by path, in one read. A path with
/// none, or whose file is newer than its origin, is absent. A path the
/// library does not hold keeps whatever origin is recorded for it.
pub fn of(conn: &Connection, paths: &[&str]) -> Result<HashMap<String, Origin>, CoreError> {
    let mut out = HashMap::new();
    for chunk in paths.chunks(500) {
        let sql = format!(
            "SELECT o.path, o.info_hash, o.torrent_name FROM file_origins o
             LEFT JOIN files f ON f.path = o.path
             WHERE o.path IN ({}) AND (f.path IS NULL OR o.recorded_at >= f.mtime)",
            placeholders(chunk.len())
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                Origin {
                    hash: r.get(1)?,
                    name: r.get(2)?,
                },
            ))
        })?;
        for row in rows {
            let (path, origin) = row?;
            out.insert(path, origin);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn an_origin_older_than_its_file_is_not_current() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("anibeam.db")).unwrap();
        store
            .write(|c| {
                c.execute_batch(
                    "INSERT INTO sources (id, path, added_at) VALUES (1, '/tv', 0);
                     INSERT INTO series (id, source_id, kind, path, folder_name, added_at) VALUES (1, 1, 'show', '/tv/Show', 'Show', 0);
                     INSERT INTO files (id, series_id, path, size, mtime, kind, number, label, episode_key, seen_at) VALUES
                         (1, 1, '/tv/Show/01.mkv', 1, 100, 'episode', 1, '01', '1', 0),
                         (2, 1, '/tv/Show/02.mkv', 1, 100, 'episode', 2, '02', '2', 0);
                     INSERT INTO file_origins (path, info_hash, torrent_name, recorded_at) VALUES
                         ('/tv/Show/01.mkv', 'aaa', 'One', 100),
                         ('/tv/Show/02.mkv', 'bbb', 'Two', 99),
                         ('/elsewhere/03.mkv', 'ccc', 'Three', 0);",
                )?;
                Ok(())
            })
            .unwrap();
        let known = store
            .read(|c| {
                of(
                    c,
                    &[
                        "/tv/Show/01.mkv",
                        "/tv/Show/02.mkv",
                        "/elsewhere/03.mkv",
                        "/none.mkv",
                    ],
                )
            })
            .unwrap();
        assert_eq!(known.len(), 2, "{known:?}");
        assert_eq!(known["/tv/Show/01.mkv"].name, "One");
        assert!(!known.contains_key("/tv/Show/02.mkv"));
        assert_eq!(known["/elsewhere/03.mkv"].hash, "ccc");
    }
}
