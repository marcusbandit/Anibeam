//! A MyAnimeList account pinned to the app id 2.5.0 shipped with (#1408):
//! launch signs it out and leaves a notice, an account with the user's own
//! id is left alone, and a Save never pins the bundled id again.

mod common;

use anibeam_core::trackers::accounts::{self, RETIRED_MAL_CLIENT_ID};
use anibeam_core::*;

fn take_notice(core: &Core) -> Option<String> {
    match core.call(Call::TakeTrackerNotice).unwrap() {
        Reply::TrackerNotice { message } => message,
        other => panic!("{other:?}"),
    }
}

/// `(profile, client_id, username, connected_at)` of one MAL row.
type MalRow = (i64, Option<String>, Option<String>, Option<i64>);

/// Every MAL row, by profile.
fn mal_rows(core: &Core) -> Vec<MalRow> {
    core.services()
        .store
        .read(|c| {
            let mut stmt = c.prepare(
                "SELECT profile_id, client_id, username, connected_at FROM tracker_accounts
                  WHERE tracker = 'mal' ORDER BY profile_id",
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap()
}

fn mal_entries(core: &Core) -> i64 {
    core.services()
        .store
        .read(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM tracker_entries WHERE tracker = 'mal'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap()
}

/// The owner's tokens and pasted secret, as a 2.5.0 sign-in left them.
fn store_secrets(core: &Core) {
    let secrets = core.services().secrets();
    secrets
        .set(&accounts::access_key(Tracker::Mal), "access")
        .unwrap();
    secrets
        .set(&accounts::refresh_key(Tracker::Mal), "refresh")
        .unwrap();
    secrets
        .set(&accounts::secret_key(Tracker::Mal), "secret")
        .unwrap();
}

fn has_secret(core: &Core, key: String) -> bool {
    core.services().secrets().has(&key)
}

fn seed(core: &Core, owner_id: &str, guest_id: &str) {
    let (owner_id, guest_id) = (owner_id.to_string(), guest_id.to_string());
    core.services()
        .store
        .write(move |c| {
            c.execute("INSERT INTO profiles (id, created_at) VALUES (2, 0)", [])?;
            c.execute(
                "INSERT INTO tracker_accounts (profile_id, tracker, user_id, username, client_id, connected_at)
                 VALUES (1, 'mal', 1, 'owner', ?1, 10), (2, 'mal', 2, 'guest', ?2, 10)",
                rusqlite::params![owner_id, guest_id],
            )?;
            c.execute_batch(
                "INSERT INTO tracker_entries (profile_id, tracker, media_id, status, fetched_at) VALUES
                     (1, 'mal', 10, 'watching', 0), (2, 'mal', 10, 'completed', 0);",
            )?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn an_account_pinned_to_the_retired_id_is_signed_out_and_told_once() {
    let (_dir, core, _c) = common::open_core();
    seed(&core, RETIRED_MAL_CLIENT_ID, RETIRED_MAL_CLIENT_ID);
    store_secrets(&core);

    assert!(accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());

    assert_eq!(
        mal_rows(&core),
        vec![(1, None, None, None), (2, None, None, None)]
    );
    assert_eq!(mal_entries(&core), 0);
    assert!(!has_secret(&core, accounts::access_key(Tracker::Mal)));
    assert!(!has_secret(&core, accounts::refresh_key(Tracker::Mal)));
    assert!(!has_secret(&core, accounts::secret_key(Tracker::Mal)));
    match core.call(Call::GetTrackers).unwrap() {
        Reply::Trackers { state } => {
            assert!(!state.mal.connected);
            assert_eq!(
                state.mal.client_id,
                accounts::bundled_client_id(Tracker::Mal).unwrap_or_default()
            );
        }
        other => panic!("{other:?}"),
    }

    let notice = take_notice(&core).expect("a notice is owed");
    assert!(notice.contains("MyAnimeList"), "{notice}");
    assert_eq!(take_notice(&core), None, "the notice is taken once");

    // Nothing matches any more, so the next launch does nothing and owes
    // nothing.
    assert!(!accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());
    assert_eq!(take_notice(&core), None);
}

#[test]
fn an_account_with_its_own_id_is_left_alone() {
    let (_dir, core, _c) = common::open_core();
    seed(&core, "my-own-id", "guest-own-id");
    store_secrets(&core);

    assert!(!accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());

    assert_eq!(
        mal_rows(&core),
        vec![
            (1, Some("my-own-id".into()), Some("owner".into()), Some(10)),
            (
                2,
                Some("guest-own-id".into()),
                Some("guest".into()),
                Some(10)
            ),
        ]
    );
    assert_eq!(mal_entries(&core), 2);
    assert!(has_secret(&core, accounts::access_key(Tracker::Mal)));
    assert!(has_secret(&core, accounts::secret_key(Tracker::Mal)));
    assert_eq!(take_notice(&core), None);
}

#[test]
fn only_a_guest_pinned_to_the_retired_id_keeps_the_owners_keyring() {
    let (_dir, core, _c) = common::open_core();
    seed(&core, "my-own-id", RETIRED_MAL_CLIENT_ID);
    store_secrets(&core);

    assert!(accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());

    assert_eq!(
        mal_rows(&core),
        vec![
            (1, Some("my-own-id".into()), Some("owner".into()), Some(10)),
            (2, None, None, None),
        ]
    );
    assert_eq!(mal_entries(&core), 1);
    assert!(has_secret(&core, accounts::access_key(Tracker::Mal)));
    assert!(has_secret(&core, accounts::secret_key(Tracker::Mal)));
}

#[test]
fn saving_the_bundled_id_stores_none() {
    let (_dir, core, _c) = common::open_core();
    let stored = |core: &Core| -> Option<String> {
        core.services()
            .store
            .read(|c| {
                Ok(c.query_row(
                    "SELECT client_id FROM tracker_accounts WHERE tracker = 'mal' AND profile_id = 1",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
    };
    let services = core.services();

    // The Trackers tab sends the id its field shows, the bundled one when
    // nothing else is stored, with the pasted secret.
    accounts::set_credentials_against(
        services,
        Tracker::Mal,
        " bundled ",
        Some("pasted"),
        Some("bundled"),
    )
    .unwrap();
    assert_eq!(stored(&core), None);
    assert!(has_secret(&core, accounts::secret_key(Tracker::Mal)));

    // Remove secret re-saves the same id.
    accounts::set_credentials_against(services, Tracker::Mal, "bundled", Some(""), Some("bundled"))
        .unwrap();
    assert_eq!(stored(&core), None);
    assert!(!has_secret(&core, accounts::secret_key(Tracker::Mal)));

    // A user's own id is still theirs to keep, and the bundled one clears it.
    accounts::set_credentials_against(services, Tracker::Mal, "mine", None, Some("bundled"))
        .unwrap();
    assert_eq!(stored(&core).as_deref(), Some("mine"));
    accounts::set_credentials_against(services, Tracker::Mal, "bundled", None, Some("bundled"))
        .unwrap();
    assert_eq!(stored(&core), None);

    // A build with no bundled id stores whatever was given.
    accounts::set_credentials_against(services, Tracker::Mal, "bundled", None, None).unwrap();
    assert_eq!(stored(&core).as_deref(), Some("bundled"));
}
