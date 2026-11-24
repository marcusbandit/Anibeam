//! The two sign-in flows, both of them ending at a loopback listener the
//! core binds itself.
//!
//! AniList is an implicit grant: the token comes back in the URL fragment,
//! which a server never sees, so the first hit is answered with a page
//! that re-issues the request with the fragment as a query. MAL is an
//! authorization code with plain PKCE, which is what MAL supports: it
//! rejects S256 outright, so the challenge is the verifier. MAL is treated
//! as a public client, the id and the verifier and nothing else, unless a
//! secret has been pasted or bundled for an app registered as "web".
//!
//! Carried from Electron's `trackerHandler.ts`: the urls character for
//! character and the five minute window. Unlike Electron, the browser's
//! last request is held open until the account is known, the token
//! validated or the code exchanged and the profile read, and only then
//! answered: a tab that says "connected" while the app says the sign-in
//! failed is the one outcome worse than a second of waiting.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::Trackers;
use crate::accounts::{self, Tokens};
use crate::{TRACKER_TIMEOUT, cache};
use anibeam_contract::i18n::tr;
use anibeam_contract::*;
use anibeam_foundation::jobs::{Finished, JobCtx};
use anibeam_foundation::percent::decode;
use anibeam_foundation::time;
use anibeam_net::anilist::VIEWER_QUERY;

/// The one path the listener serves. Both providers redirect here.
const CALLBACK_PATH: &str = "/callback";

/// AniList's authorize endpoint runs Laravel Passport with strict
/// parameter validation: sending `redirect_uri`, `state` or `scope`, even
/// when they match the registration, gets a generic
/// `unsupported_grant_type` back. The docs' own example sends the two
/// parameters below and nothing else, so that is what this sends.
const ANILIST_AUTHORIZE: &str = "https://anilist.co/api/v2/oauth/authorize";
const MAL_AUTHORIZE: &str = "https://myanimelist.net/v1/oauth2/authorize";
const MAL_ME: &str = "https://api.myanimelist.net/v2/users/@me";

/// How long the listener waits for the browser to come back. Long enough
/// to make an account and sign in, short enough that a forgotten tab does
/// not hold the port for the rest of the session.
const FLOW_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The state token and the PKCE verifier, in bytes before base64url.
const STATE_BYTES: usize = 16;
const VERIFIER_BYTES: usize = 32;

/// A request head longer than this is not a callback, it is somebody
/// filling the socket, so the read stops there.
const MAX_REQUEST: usize = 8 * 1024;

/// How long one connection has to send its request line. Connections are
/// served one at a time, and a browser routinely opens a socket it then
/// says nothing on, so a connection that stays silent is dropped rather
/// than allowed to hold the whole flow until it times out.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a page may take to reach the browser before the listener gives
/// up on that connection and goes back to accepting.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// What the pages call each provider. The site's own name, so the tab a
/// user is looking at says what they just signed in to; `Tracker::label`
/// says MAL, which is the shorthand the rest of the interface uses.
pub(crate) fn site_name(t: Tracker) -> &'static str {
    match t {
        Tracker::Anilist => "AniList",
        Tracker::Mal => "MyAnimeList",
    }
}

// The call -------------------------------------------------------------------

/// Starts the ConnectTracker job. The credentials are resolved here, on
/// the calling thread, so a connect with no client id fails the call
/// rather than a job nobody is watching.
pub fn connect(trackers: &Arc<Trackers>, t: Tracker) -> Result<u64, CoreError> {
    let (client_id, client_secret) = accounts::effective_credentials(&trackers.services, t)?;
    // A new connect supersedes whatever is in flight: the listener holds
    // one fixed port, so two flows cannot both be waiting on it, and a
    // stalled flow (a closed tab, a provider error page that never
    // redirected) must not lock the user out of trying again.
    if let Some(running) = trackers.services.jobs.running(JobKind::ConnectTracker) {
        let _ = trackers.services.jobs.cancel(running);
    }
    let owner = trackers.clone();
    let port = trackers.oauth_port.load(Ordering::SeqCst);
    Ok(owner
        .services
        .jobs
        .clone()
        .start(JobKind::ConnectTracker, move |ctx| async move {
            run(owner, ctx, t, client_id, client_secret, port).await
        }))
}

/// What the browser came back with, once the pages have been served.
enum Callback {
    Anilist {
        access_token: String,
        expires_at: Option<SystemTime>,
    },
    Mal {
        code: String,
    },
}

async fn run(
    trackers: Arc<Trackers>,
    ctx: Arc<JobCtx>,
    t: Tracker,
    client_id: String,
    client_secret: Option<String>,
    port: u16,
) -> Result<Finished, CoreError> {
    // First, before anything is generated or announced: a port that is
    // already taken means there is nothing to announce.
    let listener = bind_with_retry(port).await?;
    // The port the listener really has: the same one, or the one the system
    // picked when a test asked for port 0.
    let port = listener.local_addr().map_err(io_error)?.port();
    let state = random_token(STATE_BYTES);
    let verifier = random_token(VERIFIER_BYTES);
    let redirect_url = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let open_url = auth_url(t, &client_id, &state, &verifier, &redirect_url);
    // The line the activity log keeps, with no url in it.
    ctx.say(
        Level::Info,
        anibeam_contract::words!("core-trackers-oauth-waiting", site = site_name(t)),
        format!("{} sign-in: waiting for the browser", site_name(t)),
        EventBody::Notice,
    );
    // Debug on purpose: the bus persists the body of every event above
    // Debug, and MAL's authorize url carries the PKCE verifier as its
    // `code_challenge`, so an Info event here would write a secret into
    // the events table. A Debug event still reaches every subscriber, and
    // the shell that has to open the url is subscribed live.
    ctx.emit(
        Level::Debug,
        format!(
            "{} authorize url ready, waiting on {redirect_url}",
            site_name(t)
        ),
        EventBody::AuthUrlReady {
            tracker: t,
            open_url,
            redirect_url: redirect_url.clone(),
        },
    );

    let (callback, stream) = tokio::time::timeout(FLOW_TIMEOUT, serve(&listener, t, &state))
        .await
        .map_err(|_| accounts::tracker_error(t, tr("core-trackers-oauth-timed-out", &[])))??;
    let mut tab = HeldTab(Some(stream), t);

    // The browser's request is held while this runs, so the page it gets
    // says what actually happened: a provider refusal reaches the tab as the
    // error page with the provider's own words, not as a success the app
    // then contradicts. Held for so long and no longer: a limiter backing
    // off a run of 429s could outlast the browser's own patience, so past
    // the hold the tab is let go with a page that sends the user back to
    // Anibeam, and the sign-in carries on to an outcome the app reports.
    let hold = Duration::from_millis(trackers.sign_in_hold_ms.load(Ordering::SeqCst));
    let signing_in = sign_in(
        &trackers,
        t,
        callback,
        &verifier,
        &client_id,
        client_secret.as_deref(),
        &redirect_url,
    );
    tokio::pin!(signing_in);
    let signed_in = match tokio::time::timeout(hold, &mut signing_in).await {
        Ok(signed_in) => {
            match &signed_in {
                Ok(username) => tab.answer(OK, &page(Page::Connected(t, username))).await,
                Err(e) => {
                    tab.answer(BAD_REQUEST, &page(Page::Failed(t, &page_message(e))))
                        .await
                }
            }
            signed_in
        }
        Err(_) => {
            tab.answer(OK, &page(Page::Finishing(t))).await;
            signing_in.await
        }
    };
    let username = signed_in?;
    let now = accounts::state_async(&trackers.services).await?;
    ctx.emit(
        Level::Debug,
        "trackers changed",
        EventBody::TrackersChanged { state: now },
    );
    // The account has a list and the core has none of it, so the fetch is
    // started here rather than waited for by the first page that wants a
    // number. Forced, because a connection that has just happened cannot
    // be inside anybody's freshness window.
    cache::start_refresh(&trackers, Some(t), true);
    Ok(Finished {
        plain: Some(anibeam_contract::words!(
            "core-trackers-oauth-connected",
            site = site_name(t),
            username = &username
        )),
        level: Level::Info,
        message: format!("{} connected as {username}", site_name(t)),
        body: EventBody::TrackerConnected {
            tracker: t,
            username,
        },
    })
}

/// What the browser came back with, turned into a stored account: the
/// AniList token validated with a `Viewer` query, or the MAL code exchanged
/// and the profile read, then the tokens and the row saved. The username
/// is what the finished job reports.
async fn sign_in(
    trackers: &Trackers,
    t: Tracker,
    callback: Callback,
    verifier: &str,
    client_id: &str,
    client_secret: Option<&str>,
    redirect_url: &str,
) -> Result<String, CoreError> {
    let (user_id, username, tokens) = match callback {
        Callback::Anilist {
            access_token,
            expires_at,
        } => {
            let profile = trackers
                .net
                .anilist
                .graphql_within(
                    VIEWER_QUERY,
                    serde_json::json!({}),
                    Some(&access_token),
                    TRACKER_TIMEOUT,
                )
                .await?;
            (
                profile["Viewer"]["id"].as_u64(),
                name_of(t, &profile["Viewer"])?,
                Tokens {
                    access_token,
                    refresh_token: None,
                    expires_at,
                },
            )
        }
        Callback::Mal { code } => {
            let tokens = exchange(
                trackers,
                &code,
                verifier,
                client_id,
                client_secret,
                redirect_url,
            )
            .await?;
            let profile = trackers
                .net
                .mal
                .get_within(MAL_ME, &tokens.access_token, TRACKER_TIMEOUT)
                .await?;
            if !profile.is_success() {
                return Err(CoreError::provider(
                    Provider::Mal,
                    Some(u32::from(profile.status)),
                    profile.text(),
                ));
            }
            let profile: serde_json::Value = profile.json()?;
            (profile["id"].as_u64(), name_of(t, &profile)?, tokens)
        }
    };

    let store = accounts::save_tokens(&trackers.services, t, &tokens).await?;
    accounts::save_connection(
        &trackers.services,
        t,
        user_id,
        &username,
        client_id,
        tokens.expires_at,
        store,
    )
    .await?;
    Ok(username)
}

/// The name on a profile the provider sent back: a sign-in without one is
/// the provider's failure, not an account with no name.
fn name_of(t: Tracker, profile: &serde_json::Value) -> Result<String, CoreError> {
    profile["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| accounts::tracker_error(t, tr("core-trackers-oauth-no-name", &[])))
}

/// The line the browser's error page shows for a sign-in that failed after
/// the callback: the provider's own words without the log's prefix.
fn page_message(e: &CoreError) -> String {
    match e {
        CoreError::Provider { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// The code exchange. The verifier is what proves this is the client that
/// started the flow; the secret rides along only for an app registered as
/// "web", and a refusal for want of one says so in words a user can act on.
async fn exchange(
    trackers: &Trackers,
    code: &str,
    verifier: &str,
    client_id: &str,
    client_secret: Option<&str>,
    redirect_url: &str,
) -> Result<Tokens, CoreError> {
    let form = accounts::token_form(
        client_id,
        client_secret,
        [
            ("code", code),
            ("code_verifier", verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_url),
        ],
    );
    let response = trackers
        .net
        .mal
        .post_form_within(accounts::MAL_TOKEN_URL, form, TRACKER_TIMEOUT)
        .await?;
    if !response.is_success() {
        return Err(accounts::token_failure(&response, client_secret.is_some()));
    }
    accounts::parse_tokens(&response.body, time::now())
}

// The listener ---------------------------------------------------------------

/// The loopback socket, bound through std so the listener exists before
/// the authorize url is announced, then handed to tokio. std sets
/// `SO_REUSEADDR` on Unix, so a socket still in `TIME_WAIT` from the last
/// flow does not refuse the next one.
fn bind(port: u16) -> Result<TcpListener, CoreError> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
        tracing::debug!("the loopback listener could not bind {port}: {e}");
        let service = tr("core-trackers-oauth-could-not-start", &[]);
        if e.kind() != std::io::ErrorKind::AddrInUse {
            return CoreError::service(
                service,
                "listen_failed",
                tr(
                    "core-trackers-oauth-port-failed",
                    &[
                        ("port", port.to_string().into()),
                        ("error", e.to_string().into()),
                    ],
                ),
            );
        }
        // The port sits in the range the system hands out for any program's
        // connections, so a passing one can hold it for a moment, and a
        // program listening there holds it until it stops (#456). The
        // registered redirect pins the port, so waiting or closing the other
        // program is the remedy. Not `Io`, which would call it a file error.
        CoreError::service(
            service,
            "port_busy",
            tr(
                "core-trackers-oauth-port-busy",
                &[("port", port.to_string().into())],
            ),
        )
    })?;
    listener.set_nonblocking(true).map_err(io_error)?;
    TcpListener::from_std(listener).map_err(io_error)
}

/// The bind, retried while the port is still held: up to two seconds, a
/// tenth of a second apart. A new connect cancels the one in flight, and
/// that flow's listener is dropped by its own task, so the caller can and
/// does get here first. Two seconds is far longer than a drop takes and
/// far shorter than a user would wait to be told about a real clash.
async fn bind_with_retry(port: u16) -> Result<TcpListener, CoreError> {
    const GAP: Duration = Duration::from_millis(100);
    const ATTEMPTS: u32 = 20;
    for _ in 1..ATTEMPTS {
        if let Ok(listener) = bind(port) {
            return Ok(listener);
        }
        tokio::time::sleep(GAP).await;
    }
    bind(port)
}

/// One connection at a time until the flow is settled. Everything that is
/// not the callback the core is waiting for is answered and the loop goes
/// round again, so a favicon request or a stray reload cannot end a flow.
/// The callback itself is not answered here: its connection comes back
/// with it, and the page goes out once the sign-in has an outcome.
async fn serve(
    listener: &TcpListener,
    t: Tracker,
    state: &str,
) -> Result<(Callback, TcpStream), CoreError> {
    loop {
        let (mut stream, _) = listener.accept().await.map_err(io_error)?;
        let target = match read_target(&mut stream).await {
            Some(target) => target,
            // Not a GET, or nothing readable at all. Answered rather than
            // dropped, so the client is never left hanging.
            None => {
                reply(&mut stream, NOT_FOUND, "not found").await;
                continue;
            }
        };
        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
        if path != CALLBACK_PATH {
            reply(&mut stream, NOT_FOUND, "not found").await;
            continue;
        }
        let params = query_pairs(query);
        let get = |name: &str| {
            params
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };

        // AniList lands here first with the token still in the fragment.
        // Neither parameter present is that hop, and the page below reissues
        // the request with the fragment as a query the listener can read.
        if t == Tracker::Anilist && get("access_token").is_none() && get("error").is_none() {
            reply(&mut stream, OK, &forwarder(t)).await;
            continue;
        }
        if let Some(error) = get("error") {
            reply(&mut stream, BAD_REQUEST, &page(Page::Failed(t, &error))).await;
            return Err(accounts::tracker_error(
                t,
                tr(
                    "core-trackers-oauth-provider-error",
                    &[
                        ("site", site_name(t).into()),
                        ("error", error.as_str().into()),
                    ],
                ),
            ));
        }
        if t == Tracker::Mal {
            // MAL carries the state back; AniList is never sent one, so it
            // is only checked where it was sent.
            if get("state").as_deref() != Some(state) {
                return Err(refuse(stream, t, "core-trackers-oauth-state-mismatch").await);
            }
            let Some(code) = get("code") else {
                return Err(refuse(stream, t, "core-trackers-oauth-no-code").await);
            };
            return Ok((Callback::Mal { code }, stream));
        }
        let Some(access_token) = get("access_token") else {
            return Err(refuse(stream, t, "core-trackers-oauth-no-token").await);
        };
        let expires_at = get("expires_in")
            .and_then(|seconds| seconds.parse::<u64>().ok())
            .map(|seconds| time::now() + Duration::from_secs(seconds));
        return Ok((
            Callback::Anilist {
                access_token,
                expires_at,
            },
            stream,
        ));
    }
}

/// A callback that cannot sign anyone in: the tab and the error both say
/// why, in the catalog's words under `id`.
async fn refuse(mut stream: TcpStream, t: Tracker, id: &str) -> CoreError {
    let message = tr(id, &[]);
    reply(&mut stream, BAD_REQUEST, &page(Page::Failed(t, &message))).await;
    accounts::tracker_error(t, message)
}

/// The request target off the first line, reading until the blank line
/// that ends the headers. Nothing here trusts what arrives: a client that
/// never sends that blank line stops at 8 KB or at the read timeout, and a
/// first line that is not a GET of a target is simply not a callback.
/// Every one of those is `None`, never a failure: one bad connection must
/// not end a sign-in the user is halfway through.
async fn read_target(stream: &mut TcpStream) -> Option<String> {
    let head = match tokio::time::timeout(READ_TIMEOUT, read_head(stream)).await {
        Ok(Ok(head)) => head,
        Ok(Err(e)) => {
            tracing::debug!("the callback connection could not be read: {e}");
            return None;
        }
        Err(_) => {
            tracing::debug!("the callback connection sent nothing within {READ_TIMEOUT:?}");
            return None;
        }
    };
    let text = String::from_utf8_lossy(&head);
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) => Some(target.to_string()),
        _ => None,
    }
}

async fn read_head(stream: &mut TcpStream) -> Result<Vec<u8>, std::io::Error> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while head.len() < MAX_REQUEST && !ends_headers(&head) {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    Ok(head)
}

fn ends_headers(head: &[u8]) -> bool {
    head.windows(4).any(|w| w == b"\r\n\r\n")
}

const OK: &str = "200 OK";
const BAD_REQUEST: &str = "400 Bad Request";
const NOT_FOUND: &str = "404 Not Found";

/// The HTTP head of every reply the listener sends.
fn http_head(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
}

/// One reply, one connection. The browser reads to the end of the body and
/// the socket closes behind it, which is what `Connection: close` promises
/// and what makes the next hit of the flow a fresh accept.
///
/// Best effort by nature: the page is a courtesy to whoever is looking at
/// the tab, so a client that has already gone away is worth a log line and
/// nothing more. The flow itself turns on what the request carried.
async fn reply(stream: &mut TcpStream, status: &str, body: &str) {
    // Bounded: a page is a few hundred kilobytes, more than a socket's
    // buffers hold, so a client that connects and never reads would
    // otherwise hold the one-at-a-time listener until the flow times out.
    match tokio::time::timeout(
        WRITE_TIMEOUT,
        write_all(stream, &http_head(status, body), body),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::debug!("the callback page could not be written: {e}"),
        Err(_) => tracing::debug!("the callback page was not read within {WRITE_TIMEOUT:?}"),
    }
    // A client reading to EOF needs the write half closed before it sees
    // the page as complete.
    let _ = stream.shutdown().await;
}

/// The browser's callback request while the sign-in it carried is under
/// way. Answered once, through `answer`. A job cancelled or superseded in
/// the meantime is dropped mid-await and never reaches `answer`, so the
/// drop answers instead, with the cancelled page, rather than leaving the
/// tab with a reset connection. That write is synchronous, because a drop
/// has no executor to await on: the page goes onto a loopback socket the
/// browser is reading, and the write timeout bounds the wait if it is not.
struct HeldTab(Option<TcpStream>, Tracker);

impl HeldTab {
    async fn answer(&mut self, status: &str, body: &str) {
        if let Some(mut stream) = self.0.take() {
            reply(&mut stream, status, body).await;
        }
    }
}

impl Drop for HeldTab {
    fn drop(&mut self) {
        let Some(stream) = self.0.take() else { return };
        let Ok(mut stream) = stream.into_std() else {
            return;
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        let body = page(Page::Cancelled(self.1));
        use std::io::Write;
        let written = stream
            .write_all(http_head(OK, &body).as_bytes())
            .and_then(|_| stream.write_all(body.as_bytes()));
        if let Err(e) = written {
            tracing::debug!("the cancelled page could not be written: {e}");
        }
        let _ = stream.shutdown(std::net::Shutdown::Write);
    }
}

async fn write_all(stream: &mut TcpStream, head: &str, body: &str) -> Result<(), std::io::Error> {
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.flush().await
}

fn io_error(e: std::io::Error) -> CoreError {
    CoreError::Io {
        path: None,
        message: e.to_string(),
    }
}

// Urls and encoding ----------------------------------------------------------

fn auth_url(
    t: Tracker,
    client_id: &str,
    state: &str,
    verifier: &str,
    redirect_url: &str,
) -> String {
    match t {
        Tracker::Anilist => format!(
            "{ANILIST_AUTHORIZE}?client_id={}&response_type=token",
            encode(client_id)
        ),
        // Plain PKCE: MAL does not support S256, so the challenge is the
        // verifier, which is what their own documentation shows.
        Tracker::Mal => format!(
            "{MAL_AUTHORIZE}?response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=plain",
            encode(client_id),
            encode(redirect_url),
            encode(state),
            encode(verifier)
        ),
    }
}

/// A fresh state or verifier: random bytes as base64url with no padding,
/// which is both what the providers accept in a query and what Electron
/// sent.
fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Percent-encoding of one query value, RFC 3986 unreserved characters
/// left alone. base64url output passes through untouched, which is why the
/// state and the verifier read the same on the wire as they do in memory.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The query as pairs, decoded. A pair with no `=` is a name with an empty
/// value, which is how a browser sends a bare flag.
fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(name), decode(value))
        })
        .collect()
}

// The pages ------------------------------------------------------------------

/// The look every page shares: an eyecatch, the card an anime cuts to
/// between its halves. The logo's ア with its speed lines runs off the edge
/// of the tab, and the outcome is set huge in Archivo, the site's face.
/// Every page sits on the app's own ground, Anibeam Dark or Light after the
/// browser; a finished sign-in paints the ア in Anibeam's yellow, never the
/// whole tab. One motion: the ア cuts in once, like a smear frame.
///
/// Nothing is fetched. The icon is inline and Archivo rides along as a data
/// url, so the type is the same on every machine whether or not Archivo is
/// installed; that puts a page at about 250 kilobytes, which the loopback
/// socket hands to the browser in a moment.
const PAGE_STYLE: &str = r#"
:root{--ground:#07090f;--fg:#f2f5fa;--fg-2:#a3aec5;--art:#161c28;--state:#fdc901;color-scheme:dark}
@media (prefers-color-scheme:light){:root{--ground:#f4f6fb;--fg:#0e1220;--fg-2:#4a536a;--art:#e3e8f1;color-scheme:light}}
.ok{--state:#fdc901;--art:#fdc901}
.bad{--state:#f0718a}.off{--state:var(--fg-2)}
@media (prefers-color-scheme:light){.bad{--state:#c8405e}.ok,.wait,.go{--state:var(--fg)}}
*{box-sizing:border-box}
html{background:var(--ground);overflow-x:clip}
body{margin:0;min-height:100vh;min-height:100dvh;display:flex;flex-direction:column;justify-content:space-between;
gap:3rem;padding:clamp(20px,5vw,64px);overflow-x:clip;position:relative;background:var(--ground);color:var(--fg);
font:400 18px/1.5 Archivo,system-ui,sans-serif}
.art{position:absolute;z-index:0;right:-14vmin;top:50%;width:min(82vmin,780px);color:var(--art);
transform:translateY(-55%) rotate(-6deg);animation:cut .42s cubic-bezier(.2,.9,.1,1) both}
.art svg{width:100%;height:auto;display:block}.art rect{display:none}.art path{fill:currentColor}
header,main{position:relative;z-index:1}
header{display:flex;align-items:center;gap:.6rem;font-weight:700;font-size:1.05rem;letter-spacing:-.01em}
header svg{width:30px;height:30px}
main{max-width:min(100%,62rem)}
.word{display:block;margin:0 0 .12em -.04em;color:var(--state);font-weight:900;font-stretch:125%;
font-size:clamp(2.4rem,11.5vw,11rem);line-height:.86;letter-spacing:-.045em;text-wrap:balance;
-webkit-text-stroke:.07em var(--ground);paint-order:stroke fill}
h1{margin:0;max-width:24ch;font-size:clamp(1.5rem,3.2vw,2.4rem);line-height:1.12;font-weight:650;font-stretch:105%;
letter-spacing:-.02em;text-wrap:balance}
p{margin:.9rem 0 0;max-width:38ch;color:var(--fg-2);text-wrap:pretty}
.who{color:var(--fg);font-size:clamp(1.1rem,1.8vw,1.35rem)}
.who b{font-weight:800}
.why{font:500 .95rem/1.4 ui-monospace,monospace;color:var(--fg);overflow-wrap:anywhere}
@keyframes cut{from{transform:translate(45%,-55%) rotate(-6deg) skewX(-18deg);filter:blur(6px)}}
@media (max-width:640px){.art{width:92vmin;right:-30vmin;top:28%}}
@media (prefers-reduced-motion:reduce){.art{animation:none}}
"#;

/// Archivo, the face the site and the shell set Anibeam in, latin and
/// latin-ext, copied from `site/fonts` so the crate stands on its own.
/// Its licence is in `assets/OFL-Archivo.txt`.
const ARCHIVO: [(&[u8], &str); 2] = [
    (
        include_bytes!("../assets/Archivo-latin-100-900.woff2"),
        "U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,U+0329,\
         U+2000-206F,U+20AC,U+2122,U+2191,U+2193,U+2212,U+2215,U+FEFF,U+FFFD",
    ),
    (
        include_bytes!("../assets/Archivo-latin-ext-100-900.woff2"),
        "U+0100-02BA,U+02BD-02C5,U+02C7-02CC,U+02CE-02D7,U+02DD-02FF,U+0304,U+0308,U+0329,\
         U+1D00-1DBF,U+1E00-1E9F,U+1EF2-1EFF,U+2020,U+20A0-20AB,U+20AD-20C4,U+2113,U+2C60-2C7F,U+A720-A7FF",
    ),
];

/// The `@font-face` rules for [`ARCHIVO`], built once.
fn font_faces() -> &'static str {
    static FACES: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        ARCHIVO
            .iter()
            .map(|(bytes, range)| {
                format!(
                    "@font-face{{font-family:Archivo;font-weight:100 900;font-stretch:62% 125%;\
                     src:url(data:font/woff2;base64,{}) format('woff2');unicode-range:{range}}}",
                    STANDARD.encode(bytes)
                )
            })
            .collect()
    });
    &FACES
}

/// The app's icon, inline. The header shows it whole; the art behind the
/// words is its ア alone.
/// A copy of `apps/linux/assets/icon.svg`, kept in the crate like the fonts.
const ICON: &str = include_str!("../assets/icon.svg");

/// The tabs the listener leaves behind, each for the tracker it belongs to.
enum Page<'a> {
    /// A finished sign-in, with the account it signed in as. The tab asks
    /// the browser to close it a beat later; a browser that refuses a script
    /// closing a tab it did not open leaves the page saying the same thing
    /// in words.
    Connected(Tracker, &'a str),
    /// A sign-in that failed, with the words why.
    Failed(Tracker, &'a str),
    /// The tab let go before the sign-in finished. Neither a success nor a
    /// failure, because neither is known yet; the app is where the outcome
    /// lands.
    Finishing(Tracker),
    /// A sign-in cancelled, or replaced by a newer one, while it was being
    /// finished.
    Cancelled(Tracker),
}

/// One page. Its words come from the catalog, which are the app's own and go
/// in as they are. The username and the failure's words are escaped first:
/// the failure comes off the query the browser was redirected with, and
/// nothing a provider or a crafted link says should be able to write markup
/// here.
fn page(kind: Page) -> String {
    let (t, words, state, extra, close) = match kind {
        Page::Connected(t, username) => {
            let who = format!(
                "<p class=\"who\">{}</p>",
                tr(
                    "core-trackers-page-signed-in",
                    &[("username", format!("<b>{}</b>", escape(username)).into())]
                )
            );
            (t, CONNECTED, "ok", who, true)
        }
        Page::Failed(t, why) => (
            t,
            FAILED,
            "bad",
            format!("<p class=\"why\">{}</p>", escape(why)),
            false,
        ),
        Page::Finishing(t) => (t, FINISHING, "wait", String::new(), false),
        Page::Cancelled(t) => (t, CANCELLED, "off", String::new(), false),
    };
    let close = match close {
        true => "<script>setTimeout(function(){window.close()},1500)</script>",
        false => "",
    };
    shell(t, words, state, &extra, close)
}

/// The page around a set of words: `[title, status, heading, closing line]`
/// as catalog ids, the state class, anything that goes between the heading
/// and the closing line, and a trailing script.
fn shell(
    t: Tracker,
    [title, status, heading, line]: Words,
    state: &str,
    extra: &str,
    script: &str,
) -> String {
    let words = |id: &str| {
        tr(
            id,
            &[("site", site_name(t).into()), ("app", "Anibeam".into())],
        )
    };
    let line = match line {
        "" => String::new(),
        id => format!("<p class=\"line\">{}</p>", words(id)),
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{}</title><style>{}{PAGE_STYLE}</style></head>\
         <body class=\"{state}\"><div class=\"art\" aria-hidden=\"true\">{ICON}</div>\
         <header>{ICON}Anibeam</header>\
         <main><strong class=\"word\">{}</strong><h1>{}</h1>{extra}{line}</main>{script}</body></html>",
        words(title),
        font_faces(),
        words(status),
        words(heading),
    )
}

/// A page's words as catalog ids: [title, status, heading, closing line].
type Words = [&'static str; 4];

const CONNECTED: Words = [
    "core-trackers-page-connected",
    "core-trackers-page-status-connected",
    "core-trackers-page-connected",
    "core-trackers-page-close-tab",
];
const FAILED: Words = [
    "core-trackers-page-failed-title",
    "core-trackers-page-status-failed",
    "core-trackers-page-failed",
    "core-trackers-page-go-back",
];
const FINISHING: Words = [
    "core-trackers-page-finishing-title",
    "core-trackers-page-status-finishing",
    "core-trackers-page-finishing",
    "core-trackers-page-finishing-body",
];
const CANCELLED: Words = [
    "core-trackers-page-cancelled-title",
    "core-trackers-page-status-cancelled",
    "core-trackers-page-cancelled",
    "core-trackers-page-cancelled-body",
];
const CONNECTING: Words = [
    "core-trackers-page-connecting-title",
    "core-trackers-page-status-connecting",
    "core-trackers-page-connecting",
    "",
];

/// The implicit-grant hop: the fragment never reaches the server, so this
/// page reads it in the browser and reloads with the same parameters as a
/// query, which the listener can read. It wears the same look for the
/// moment it is on screen, and turns into the failure when there is no
/// fragment to forward.
fn forwarder(t: Tracker) -> String {
    let none = tr("core-trackers-page-no-token", &[("app", "Anibeam".into())]);
    let failed = tr("core-trackers-page-status-failed", &[]);
    let heading = tr("core-trackers-page-failed", &[]);
    // A JSON string is a JavaScript string literal, and the words go in as
    // text, so nothing in the catalog can break out of the script.
    // `<` is escaped as well, so no `</script>` in a translation can end
    // the element.
    let quote = |s: &str| {
        serde_json::to_string(s)
            .unwrap_or_default()
            .replace('<', "\\u003c")
    };
    let script = format!(
        "<script>(function(){{var h=window.location.hash;\
         if(!h){{document.body.className='bad';document.querySelector('.word').textContent={failed};\
         document.querySelector('h1').textContent={heading};\
         var p=document.createElement('p');p.textContent={none};document.querySelector('main').appendChild(p);return;}}\
         window.location.replace(window.location.pathname+'?'+h.slice(1));}})();</script>",
        failed = quote(&failed),
        none = quote(&none),
        heading = quote(&heading),
    );
    shell(t, CONNECTING, "go", "", &script)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Any program's loopback connection can sit on the sign-in port, so a
    /// busy port says what to do and never sends anyone looking for a
    /// second Anibeam.
    #[tokio::test]
    async fn a_busy_port_says_so_and_asks_for_another_try() {
        let holder = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = holder.local_addr().unwrap().port();
        let err = bind(port).expect_err("a held port must fail the bind");
        assert!(matches!(&err, CoreError::Service { code, .. } if code == "port_busy"));
        // What the person reads, prefix and all.
        let shown = err.to_string();
        assert!(!shown.to_lowercase().contains("anibeam"), "{shown}");
        assert!(!shown.contains("file"), "{shown}");
        assert!(shown.starts_with("Could not start sign-in: "), "{shown}");
        assert!(shown.contains(&format!("port {port}")), "{shown}");
        assert!(shown.contains("in use by another program"), "{shown}");
        assert!(shown.contains("try again"), "{shown}");
    }

    #[test]
    fn the_anilist_url_carries_two_parameters_and_no_more() {
        let url = auth_url(
            Tracker::Anilist,
            "123",
            "st",
            "vf",
            "http://127.0.0.1:53682/callback",
        );
        assert_eq!(
            url,
            "https://anilist.co/api/v2/oauth/authorize?client_id=123&response_type=token"
        );
    }

    #[test]
    fn the_mal_url_encodes_the_redirect_and_sends_a_plain_challenge() {
        let url = auth_url(
            Tracker::Mal,
            "mid",
            "st-1",
            "vf_2",
            "http://127.0.0.1:53682/callback",
        );
        assert_eq!(
            url,
            "https://myanimelist.net/v1/oauth2/authorize?response_type=code&client_id=mid\
             &redirect_uri=http%3A%2F%2F127.0.0.1%3A53682%2Fcallback&state=st-1&code_challenge=vf_2&code_challenge_method=plain"
        );
    }

    /// base64url is exactly the alphabet percent-encoding leaves alone, so
    /// a state reads the same in the url as it does in memory.
    #[test]
    fn a_random_token_is_url_safe_and_the_length_asked_for() {
        let token = random_token(16);
        assert_eq!(token, encode(&token));
        assert_eq!(URL_SAFE_NO_PAD.decode(&token).unwrap().len(), 16);
        assert_ne!(token, random_token(16));
    }

    #[test]
    fn a_query_decodes_pairs_plus_signs_and_escapes() {
        let pairs = query_pairs("code=a%2Bb&state=one+two&flag&empty=");
        assert_eq!(pairs[0], ("code".to_string(), "a+b".to_string()));
        assert_eq!(pairs[1], ("state".to_string(), "one two".to_string()));
        assert_eq!(pairs[2], ("flag".to_string(), String::new()));
        assert_eq!(pairs[3], ("empty".to_string(), String::new()));
        assert!(query_pairs("").is_empty());
    }

    #[test]
    fn the_pages_name_the_site_and_escape_what_a_provider_sent() {
        let shown = page(Page::Connected(Tracker::Mal, "<i>alex</i>"));
        assert!(shown.contains("MyAnimeList"), "{shown}");
        assert!(shown.contains("<b>&lt;i&gt;alex&lt;/i&gt;</b>"), "{shown}");
        assert!(shown.contains("window.close()"), "{shown}");
        assert!(page(Page::Connected(Tracker::Anilist, "alex")).contains("AniList"));
        let shown = page(Page::Failed(Tracker::Mal, "<script>alert(1)</script>"));
        assert!(!shown.contains("<script>alert"));
        assert!(shown.contains("&lt;script&gt;"));
        assert!(shown.contains("<body class=\"bad\">"), "{shown}");
        assert!(!shown.contains("window.close()"), "{shown}");
        let hop = forwarder(Tracker::Anilist);
        assert!(hop.contains("window.location.hash"), "{hop}");
        assert!(hop.contains("<body class=\"go\">"), "{hop}");
    }

    /// Every page is whole on its own: every catalog id resolves, nothing is
    /// fetched from anywhere, the icon is inline, and the title is plain text.
    #[test]
    fn every_page_is_self_contained_and_fully_worded() {
        let pages = [
            page(Page::Connected(Tracker::Anilist, "alex")),
            page(Page::Failed(Tracker::Anilist, "why")),
            page(Page::Finishing(Tracker::Mal)),
            page(Page::Cancelled(Tracker::Mal)),
            forwarder(Tracker::Anilist),
        ];
        for shown in &pages {
            assert!(!shown.contains("core-trackers-"), "a missing id: {shown}");
            let outside = shown
                .replace("http://www.w3.org/2000/svg", "")
                .split("url(data:")
                .map(|part| part.split_once(')').map_or(part, |(_, rest)| rest))
                .collect::<String>();
            assert!(!outside.contains("http"), "a fetch: {shown}");
            assert!(!shown.contains("@import"), "{shown}");
            assert!(shown.contains("fill=\"#FDC901\""), "the icon: {shown}");
            assert!(
                !shown.contains("<title><span"),
                "the title is plain: {shown}"
            );
            assert!(shown.len() < 320 * 1024, "{} bytes", shown.len());
        }
    }

    /// Only a finished sign-in closes its tab, and each outcome wears its
    /// own state on the beam.
    #[test]
    fn each_page_has_its_own_state_and_only_success_closes_the_tab() {
        for (shown, state) in [
            (page(Page::Connected(Tracker::Mal, "alex")), "ok"),
            (page(Page::Failed(Tracker::Mal, "why")), "bad"),
            (page(Page::Finishing(Tracker::Mal)), "wait"),
            (page(Page::Cancelled(Tracker::Mal)), "off"),
        ] {
            assert!(
                shown.contains(&format!("<body class=\"{state}\">")),
                "{shown}"
            );
            assert_eq!(shown.contains("window.close()"), state == "ok", "{shown}");
        }
    }

    /// Each page's own words in the right slot: a swap would put the
    /// heading's words in the tab's title or the status line.
    #[test]
    fn each_page_puts_its_words_in_the_right_slots() {
        let shown = page(Page::Failed(Tracker::Mal, "why"));
        let words = |id: &str| tr(id, &[("app", "Anibeam".into())]);
        assert!(
            shown.contains(&format!(
                "<title>{}</title>",
                words("core-trackers-page-failed-title")
            )),
            "{shown}"
        );
        assert!(
            shown.contains(&format!(
                "<strong class=\"word\">{}</strong><h1>{}</h1>",
                words("core-trackers-page-status-failed"),
                words("core-trackers-page-failed")
            )),
            "{shown}"
        );
    }

    #[test]
    fn the_header_end_is_found_across_a_split_read() {
        assert!(!ends_headers(b"GET /callback HTTP/1.1\r\n"));
        assert!(ends_headers(b"GET /callback HTTP/1.1\r\nHost: x\r\n\r\n"));
    }
}
