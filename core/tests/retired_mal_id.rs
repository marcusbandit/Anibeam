//! A MyAnimeList account pinned to the app id 2.5.0 shipped with (#1408):
//! launch signs it out and leaves a notice, an account with the user's own
//! id is left alone, and a Save never pins the bundled id again.

mod common;

use std::time::Duration;

use anibeam_core::trackers::accounts::{self, RETIRED_MAL_CLIENT_ID};
use anibeam_core::*;

fn notice(core: &Core) -> Option<String> {
    match core.call(Call::GetTrackerNotice).unwrap() {
        Reply::TrackerNotice { message } => message,
        other => panic!("{other:?}"),
    }
}

/// The owner's MAL client id as stored, None when the row holds none.
fn stored(core: &Core) -> Option<String> {
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
fn an_account_pinned_to_the_retired_id_is_signed_out_and_told_until_dismissed() {
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

    let line = notice(&core).expect("a notice is owed");
    assert!(line.contains("MyAnimeList"), "{line}");
    // Reading it leaves it owed: a toast nobody saw is shown next time.
    assert_eq!(notice(&core), Some(line));

    // Nothing matches any more, so the next launch does nothing new.
    assert!(!accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());

    assert_eq!(core.call(Call::DismissTrackerNotice).unwrap(), Reply::Ok);
    assert_eq!(notice(&core), None, "a dismissed notice is gone");
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
    assert_eq!(notice(&core), None);
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
    assert_eq!(
        notice(&core),
        None,
        "a guest's reset does not tell the owner"
    );
}

/// The launch itself runs the cleanup, off the calling thread, and says the
/// trackers changed when it is done.
#[test]
fn starting_the_core_signs_out_an_account_pinned_to_the_retired_id() {
    // A build that still carries the retired id leaves its own account be.
    if accounts::bundled_client_id(Tracker::Mal) == Some(RETIRED_MAL_CLIENT_ID) {
        return;
    }
    let (_dir, core, c) = common::open_core_with_http(anibeam_core::net::FakeHttp::new());
    seed(&core, RETIRED_MAL_CLIENT_ID, "guest-own-id");
    store_secrets(&core);

    core.start().unwrap();
    common::wait_for(
        &c,
        |e| matches!(&e.body, EventBody::TrackersChanged { state } if !state.mal.connected),
        Duration::from_secs(10),
    );

    assert_eq!(mal_rows(&core)[0], (1, None, None, None));
    assert!(!has_secret(&core, accounts::access_key(Tracker::Mal)));
    assert!(notice(&core).is_some());
    core.shutdown();
}

/// A signed-in account keeps the id it is given, the bundled one included:
/// its tokens were issued to that app, and a later build with another
/// bundled id must still refresh them with this one.
#[test]
fn a_signed_in_account_keeps_the_bundled_id_it_is_given() {
    let (_dir, core, _c) = common::open_core();
    core.services()
        .store
        .write(|c| {
            c.execute(
                "INSERT INTO tracker_accounts (profile_id, tracker, user_id, username, connected_at)
                 VALUES (1, 'mal', 1, 'owner', 10)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let services = core.services();

    // Save with the secret, then Remove secret, both with the field showing
    // the bundled id.
    accounts::set_credentials_against(
        services,
        Tracker::Mal,
        "bundled",
        Some("pasted"),
        Some("bundled"),
    )
    .unwrap();
    assert_eq!(stored(&core).as_deref(), Some("bundled"));
    accounts::set_credentials_against(services, Tracker::Mal, "bundled", Some(""), Some("bundled"))
        .unwrap();
    assert_eq!(stored(&core).as_deref(), Some("bundled"));

    // Signed out, the row still holds that id, so a Save keeps it too.
    core.call(Call::DisconnectTracker {
        tracker: Tracker::Mal,
    })
    .unwrap();
    accounts::set_credentials_against(services, Tracker::Mal, "bundled", None, Some("bundled"))
        .unwrap();
    assert_eq!(stored(&core).as_deref(), Some("bundled"));
}

#[test]
fn saving_the_bundled_id_stores_none() {
    let (_dir, core, _c) = common::open_core();
    let services = core.services();

    // The Trackers tab sends the id its field shows, the bundled one when
    // nothing else is stored. Alone it stores nothing.
    accounts::set_credentials_against(services, Tracker::Mal, " bundled ", None, Some("bundled"))
        .unwrap();
    assert_eq!(stored(&core), None);

    // With a pasted secret the id is kept: the secret belongs to that app,
    // and a later build must not send it with its own id.
    accounts::set_credentials_against(
        services,
        Tracker::Mal,
        " bundled ",
        Some("pasted"),
        Some("bundled"),
    )
    .unwrap();
    assert_eq!(stored(&core).as_deref(), Some("bundled"));
    assert!(has_secret(&core, accounts::secret_key(Tracker::Mal)));

    // A Save that keeps the secret keeps the id with it.
    accounts::set_credentials_against(services, Tracker::Mal, "bundled", None, Some("bundled"))
        .unwrap();
    assert_eq!(stored(&core).as_deref(), Some("bundled"));

    // Remove secret re-saves the same id, which goes with the secret.
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

#[test]
fn a_keyring_that_fails_leaves_the_owner_pinned_to_try_again() {
    // A build that still bundles the retired id has nothing to steer away from.
    if accounts::bundled_client_id(Tracker::Mal) == Some(RETIRED_MAL_CLIENT_ID) {
        return;
    }
    let (dir, core, _c) = common::open_core();
    seed(&core, RETIRED_MAL_CLIENT_ID, "guest-own-id");
    store_secrets(&core);
    // The file store rewrites through a .tmp sibling; a directory there
    // makes every write, and so every delete, fail.
    let tmp = CorePaths::under(dir.path())
        .secrets_path()
        .with_extension("json.tmp");
    std::fs::create_dir(&tmp).unwrap();

    assert!(accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());
    let owner = mal_rows(&core).into_iter().find(|r| r.0 == 1).unwrap();
    assert_eq!(owner.1.as_deref(), Some(RETIRED_MAL_CLIENT_ID));
    assert_eq!(owner.3, None, "signed out even though the keys stayed");
    assert!(notice(&core).is_some());

    // A Connect while the keyring still fails stops rather than send the
    // retired id or its secret.
    assert!(accounts::effective_credentials(core.services(), Tracker::Mal).is_err());

    // The next launch, with the keyring back, finishes the job.
    std::fs::remove_dir(&tmp).unwrap();
    assert!(accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());
    assert_eq!(stored(&core), None);
    assert!(!has_secret(&core, accounts::secret_key(Tracker::Mal)));
    assert!(!accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());
}

#[test]
fn a_connect_after_a_failed_cleanup_clears_the_retired_app_first() {
    // A build that still bundles the retired id has nothing to steer away from.
    if accounts::bundled_client_id(Tracker::Mal) == Some(RETIRED_MAL_CLIENT_ID) {
        return;
    }
    let (dir, core, _c) = common::open_core();
    seed(&core, RETIRED_MAL_CLIENT_ID, "guest-own-id");
    store_secrets(&core);
    let tmp = CorePaths::under(dir.path())
        .secrets_path()
        .with_extension("json.tmp");
    std::fs::create_dir(&tmp).unwrap();
    accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap();
    std::fs::remove_dir(&tmp).unwrap();

    // The keyring is back before the next launch and the user clicks Connect.
    if let Ok((id, secret)) = accounts::effective_credentials(core.services(), Tracker::Mal) {
        assert_ne!(id, RETIRED_MAL_CLIENT_ID);
        assert_ne!(secret.as_deref(), Some("secret"));
    }
    assert_eq!(stored(&core), None);
    assert!(!has_secret(&core, accounts::secret_key(Tracker::Mal)));
}
