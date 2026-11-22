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

/// Whether a delete of the retired app's secret is still owed.
fn owed(core: &Core) -> bool {
    core.services()
        .store
        .read(|c| {
            Ok(
                store::settings::get::<bool>(c, store::settings::MAL_RETIRED_SECRET_OWED)?
                    .unwrap_or(false),
            )
        })
        .unwrap()
}

/// The owner's pasted MAL secret, read off the store itself rather than the
/// facade's memory of which keys exist.
fn secret(core: &Core) -> Option<String> {
    core.services()
        .secrets()
        .get(&accounts::secret_key(Tracker::Mal), None)
        .unwrap()
        .map(|(value, _)| value)
}

/// Makes every write to the secrets file, and so every delete, fail: the
/// file store rewrites through a .tmp sibling, and a directory sits there.
fn break_secrets(dir: &std::path::Path) -> std::path::PathBuf {
    let tmp = CorePaths::under(dir)
        .secrets_path()
        .with_extension("json.tmp");
    std::fs::create_dir(&tmp).unwrap();
    tmp
}

#[test]
fn a_failed_delete_owes_the_secret_without_pinning_or_nagging_again() {
    // A build that still bundles the retired id has nothing to steer away from.
    if accounts::bundled_client_id(Tracker::Mal) == Some(RETIRED_MAL_CLIENT_ID) {
        return;
    }
    let (dir, core, _c) = common::open_core();
    seed(&core, RETIRED_MAL_CLIENT_ID, "guest-own-id");
    store_secrets(&core);
    let tmp = break_secrets(dir.path());

    assert!(accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());
    assert_eq!(mal_rows(&core)[0], (1, None, None, None));
    assert!(owed(&core), "the secret is owed a delete");
    assert!(notice(&core).is_some());

    // Closed with the x, it stays closed on the next launch, though the
    // delete fails again.
    assert_eq!(core.call(Call::DismissTrackerNotice).unwrap(), Reply::Ok);
    assert!(!accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());
    assert_eq!(notice(&core), None);
    assert!(owed(&core));
    assert_eq!(secret(&core).as_deref(), Some("secret"));

    // A Connect meanwhile signs in with the bundled id and never sends the
    // retired app's secret; with no bundled id there is nothing to sign in
    // with at all.
    match (
        accounts::bundled_client_id(Tracker::Mal),
        accounts::effective_credentials(core.services(), Tracker::Mal),
    ) {
        (Some(bundled), Ok((id, sent))) => {
            assert_eq!(id, bundled);
            assert_ne!(sent.as_deref(), Some("secret"));
        }
        (None, Err(_)) => {}
        other => panic!("{other:?}"),
    }
    assert!(owed(&core));

    // The next launch, with the store working, pays the debt.
    std::fs::remove_dir(&tmp).unwrap();
    assert!(!accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());
    assert!(!owed(&core));
    assert_eq!(secret(&core), None);
    assert_eq!(notice(&core), None);
}

#[test]
fn a_secret_pasted_while_one_is_owed_is_the_users_own() {
    // A build that still bundles the retired id has nothing to steer away from.
    if accounts::bundled_client_id(Tracker::Mal) == Some(RETIRED_MAL_CLIENT_ID) {
        return;
    }
    let (dir, core, _c) = common::open_core();
    seed(&core, RETIRED_MAL_CLIENT_ID, "guest-own-id");
    store_secrets(&core);
    let tmp = break_secrets(dir.path());
    accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap();
    assert!(owed(&core));
    std::fs::remove_dir(&tmp).unwrap();

    accounts::set_credentials_against(
        core.services(),
        Tracker::Mal,
        "mine",
        Some("pasted"),
        accounts::bundled_client_id(Tracker::Mal),
    )
    .unwrap();
    assert!(!owed(&core), "the stored secret is the user's now");
    assert_eq!(
        accounts::effective_credentials(core.services(), Tracker::Mal).unwrap(),
        ("mine".to_string(), Some("pasted".to_string()))
    );
}

#[test]
fn the_notice_is_not_shown_once_mal_is_connected() {
    let (_dir, core, _c) = common::open_core();
    core.services()
        .store
        .write(|c| {
            store::settings::set(c, store::settings::MAL_SIGN_IN_AGAIN, &true)?;
            c.execute(
                "INSERT INTO tracker_accounts (profile_id, tracker) VALUES (1, 'mal')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(notice(&core).is_some());

    // An import, say, connects MyAnimeList without a sign-in here.
    core.services()
        .store
        .write(|c| {
            c.execute(
                "UPDATE tracker_accounts SET user_id = 1, username = 'owner', connected_at = 10
                  WHERE tracker = 'mal' AND profile_id = 1",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(notice(&core), None);
}

/// A row pinned to an id MAL never accepted was never signed in, so the
/// tokens in the keyring are not its to drop.
#[test]
fn an_unconnected_pinned_owner_keeps_the_tokens() {
    let (_dir, core, _c) = common::open_core();
    core.services()
        .store
        .write(|c| {
            c.execute(
                "INSERT INTO tracker_accounts (profile_id, tracker, client_id) VALUES (1, 'mal', ?1)",
                [RETIRED_MAL_CLIENT_ID],
            )?;
            Ok(())
        })
        .unwrap();
    store_secrets(&core);

    assert!(accounts::forget_retired_client_id(core.services(), RETIRED_MAL_CLIENT_ID).unwrap());
    assert_eq!(stored(&core), None);
    assert!(has_secret(&core, accounts::access_key(Tracker::Mal)));
    assert!(has_secret(&core, accounts::refresh_key(Tracker::Mal)));
    assert_eq!(secret(&core), None, "the retired app's secret goes");
    assert!(!owed(&core));
    assert!(notice(&core).is_some());
}
