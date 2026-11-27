//! The usage log's collection point (#1532). An app sends its usage log here only when its
//! owner presses send: `POST /usage` with `{"send_id", "usage"}`, a fresh random id per send
//! and the log's counts. Each send is kept as one row in one SQLite-backed Durable Object
//! (`UsageObject`), size capped, shape checked and rate limited per address before the
//! object is touched. The object keeps the newest `MAX_ROWS` sends and drops the oldest.
//!
//! The owner's side, past `auth::check`: `GET /usage` reads the sends, oldest first, 100 at
//! a time (`?since=<unix ms>` only those received after then, `?after=<id>` the next page),
//! and `DELETE /usage?before=<unix ms>` drops the ones received before then. Nothing here
//! records the sender's address: the limiter counts it and forgets it.

use serde::Deserialize;
use serde_json::{Value, json};
use worker::{
    DurableObject, Env, Method, Request, RequestInit, Response, Result, State, durable_object,
};

use crate::{allowed, now_ms, query_number, text};

/// The Durable Object namespace binding in `wrangler.toml`.
const USAGE: &str = "USAGE";

/// Sends per address a minute, set in `wrangler.toml`.
const USAGE_LIMIT: &str = "USAGE_LIMIT";

/// The largest send.
pub const MAX_SEND: usize = 256 * 1024;

/// Sends the object keeps. A send past this drops the oldest, so storage has a ceiling
/// whoever posts.
const MAX_ROWS: i64 = 2000;

/// Days one send may carry. The app keeps 90.
const MAX_DAYS: usize = 100;

/// The longest key in a day's maps, and the longest header string.
const MAX_KEY: usize = 64;
const MAX_HEADER: usize = 32;

/// Rows a read answers at most; `?after=<id>` reads on.
const PAGE: i64 = 100;

/// The one object every send goes to.
const OBJECT: &str = "usage";

/// The host the Worker uses for its own requests to the object.
const INTERNAL: &str = "https://usage.internal";

/// A send as it arrives. Unknown fields are refused, so a send carries the log and its id
/// and nothing else.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Send {
    send_id: String,
    usage: Value,
}

/// Why a send is refused, as the status and the text the sender gets.
pub type Refusal = (u16, &'static str);

/// A key in a day's maps: a call name, a page id or an error kind. Letters, digits, `_`
/// and `-`, short, so a path, a title or an address never is one.
fn key_ok(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A header string: a version, `desktop`, `linux`. Short, no spaces, no slashes.
fn header_ok(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|s| {
        !s.is_empty()
            && s.len() <= MAX_HEADER
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
    })
}

/// `2026-11-27`.
fn date_ok(key: &str) -> bool {
    let b = key.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !digits(0..4) || !digits(5..7) {
        return false;
    }
    if !digits(8..10) {
        return false;
    }
    let month = (b[5] - b'0') * 10 + (b[6] - b'0');
    let day = (b[8] - b'0') * 10 + (b[9] - b'0');
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

/// A map of keys to whole counts.
fn counts_ok(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|m| m.iter().all(|(k, n)| key_ok(k) && n.is_u64()))
}

/// One day: `calls` and `pages` count maps, `errors` a count map per call, nothing else.
fn day_ok(value: &Value) -> bool {
    value.as_object().is_some_and(|m| {
        m.iter().all(|(k, v)| match k.as_str() {
            "calls" | "pages" => counts_ok(v),
            "errors" => v
                .as_object()
                .is_some_and(|m| m.iter().all(|(k, v)| key_ok(k) && counts_ok(v))),
            _ => false,
        })
    })
}

/// Whether `usage` is a usage log in format 1 and nothing more: the header strings, and
/// days by date holding only counts under short keys.
fn log_ok(usage: &Value) -> bool {
    let Some(log) = usage.as_object() else {
        return false;
    };
    log.keys().all(|k| {
        matches!(
            k.as_str(),
            "format" | "app_version" | "platform" | "os" | "days"
        )
    }) && log.get("format").and_then(Value::as_u64) == Some(1)
        && header_ok(log.get("app_version"))
        && header_ok(log.get("platform"))
        && header_ok(log.get("os"))
        && log
            .get("days")
            .and_then(Value::as_object)
            .is_some_and(|days| {
                days.len() <= MAX_DAYS && days.iter().all(|(k, v)| date_ok(k) && day_ok(v))
            })
}

/// The send's id and its log as compact JSON, or why the body is not a send: valid JSON,
/// a 32 character hex id, and a usage log in format 1 (`log_ok`).
pub fn check_send(body: &[u8]) -> std::result::Result<(String, String), Refusal> {
    if body.len() > MAX_SEND {
        return Err((413, "length limit exceeded"));
    }
    let Ok(send) = serde_json::from_slice::<Send>(body) else {
        return Err((400, "expected {\"send_id\", \"usage\"}"));
    };
    let id_ok = send.send_id.len() == 32
        && send
            .send_id
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    if !id_ok {
        return Err((400, "send_id must be 32 hex characters"));
    }
    if !log_ok(&send.usage) {
        return Err((400, "usage must be a usage log"));
    }
    Ok((send.send_id, send.usage.to_string()))
}

/// The owner's `GET /usage` and `DELETE /usage`, past `auth::check`.
pub async fn route(req: Request, env: &Env) -> Result<Response> {
    match req.method() {
        Method::Get => read(req, env).await,
        Method::Delete => prune(req, env).await,
        _ => text(405, "method not allowed"),
    }
}

/// `POST /usage`, the one route anyone may call.
pub async fn receive(mut req: Request, env: &Env) -> Result<Response> {
    if !allowed(env, USAGE_LIMIT, &req).await {
        return text(429, "too many sends, try again in a minute");
    }
    let declared = req
        .headers()
        .get("Content-Length")
        .ok()
        .flatten()
        .and_then(|len| len.parse::<usize>().ok());
    if declared.is_some_and(|len| len > MAX_SEND) {
        return text(413, "length limit exceeded");
    }
    let body = req.bytes().await?;
    let (send_id, usage) = match check_send(&body) {
        Ok(send) => send,
        Err((status, why)) => return text(status, why),
    };
    let row = json!({ "send_id": send_id, "usage": usage }).to_string();
    let mut options = RequestInit::new();
    options
        .with_method(Method::Post)
        .with_body(Some(wasm_bindgen::JsValue::from_str(&row)));
    let internal = Request::new_with_init(&format!("{INTERNAL}/store"), &options)?;
    stub(env)?.fetch_with_request(internal).await
}

async fn read(req: Request, env: &Env) -> Result<Response> {
    let url = req.url()?;
    let (Ok(since), Ok(after)) = (query_number(&url, "since"), query_number(&url, "after")) else {
        return text(400, "since and after are whole numbers");
    };
    let (since, after) = (since.unwrap_or(0), after.unwrap_or(0));
    let internal = Request::new(
        &format!("{INTERNAL}/list?since={since}&after={after}"),
        Method::Get,
    )?;
    stub(env)?.fetch_with_request(internal).await
}

async fn prune(req: Request, env: &Env) -> Result<Response> {
    let Ok(Some(before)) = query_number(&req.url()?, "before") else {
        return text(400, "expected ?before=<unix ms>");
    };
    let internal = Request::new(&format!("{INTERNAL}/prune?before={before}"), Method::Delete)?;
    stub(env)?.fetch_with_request(internal).await
}

fn stub(env: &Env) -> Result<worker::Stub> {
    env.durable_object(USAGE)?.id_from_name(OBJECT)?.get_stub()
}

/// One row as the object hands it back.
#[derive(Deserialize)]
struct Row {
    id: i64,
    at: i64,
    send_id: String,
    body: String,
}

/// A number the Worker already checked, read back off its own internal request.
fn internal_number(req: &Request, key: &str) -> Result<i64> {
    Ok(query_number(&req.url()?, key).ok().flatten().unwrap_or(0))
}

/// Every send, one row each.
#[durable_object]
pub struct UsageObject {
    state: State,
}

impl DurableObject for UsageObject {
    fn new(state: State, _env: Env) -> Self {
        let made = state.storage().sql().exec(
            "CREATE TABLE IF NOT EXISTS sends (
                id INTEGER PRIMARY KEY,
                at INTEGER NOT NULL,
                send_id TEXT NOT NULL UNIQUE,
                body TEXT NOT NULL
            )",
            None,
        );
        if let Err(err) = made {
            worker::console_error!("the usage table could not be made: {err}");
        }
        Self { state }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        let sql = self.state.storage().sql();
        match (req.method(), path.as_str()) {
            (Method::Post, "/store") => {
                #[derive(Deserialize)]
                struct Store {
                    send_id: String,
                    usage: String,
                }
                let row: Store = req.json().await?;
                let cursor = sql.exec(
                    "INSERT OR IGNORE INTO sends (at, send_id, body) VALUES (?, ?, ?)",
                    vec![
                        (now_ms() as i64).into(),
                        row.send_id.into(),
                        row.usage.into(),
                    ],
                )?;
                // Twice the same id is one send: the second changes nothing
                if cursor.rows_written() == 0 {
                    return text(200, "already stored");
                }
                // The newest MAX_ROWS stay. With fewer, the inner select is empty and
                // nothing goes.
                sql.exec(
                    "DELETE FROM sends WHERE id < \
                     (SELECT id FROM sends ORDER BY id DESC LIMIT 1 OFFSET ?)",
                    vec![(MAX_ROWS - 1).into()],
                )?;
                text(201, "stored")
            }
            (Method::Get, "/list") => {
                let since = internal_number(&req, "since")?;
                let after = internal_number(&req, "after")?;
                let rows: Vec<Row> = sql
                    .exec(
                        "SELECT id, at, send_id, body FROM sends \
                         WHERE at > ? AND id > ? ORDER BY id LIMIT ?",
                        vec![since.into(), after.into(), PAGE.into()],
                    )?
                    .to_array()?;
                let next = (rows.len() as i64 == PAGE).then(|| rows.last().map(|r| r.id));
                let sends: Vec<Value> = rows
                    .into_iter()
                    .map(|r| {
                        json!({
                            "id": r.id,
                            "at": r.at,
                            "send_id": r.send_id,
                            "usage": serde_json::from_str::<Value>(&r.body).unwrap_or(Value::Null),
                        })
                    })
                    .collect();
                Response::from_json(&json!({ "sends": sends, "next": next.flatten() }))
            }
            (Method::Delete, "/prune") => {
                let before = internal_number(&req, "before")?;
                let cursor = sql.exec("DELETE FROM sends WHERE at < ?", vec![before.into()])?;
                Response::from_json(&json!({ "deleted": cursor.rows_written() }))
            }
            _ => text(404, "not found"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(id: &str, usage: Value) -> Vec<u8> {
        json!({ "send_id": id, "usage": usage })
            .to_string()
            .into_bytes()
    }

    fn log() -> Value {
        json!({ "format": 1, "app_version": "2.6.0", "platform": "desktop", "os": "linux",
                "days": { "2026-11-27": { "calls": { "Scan": 2 } } } })
    }

    const ID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn a_send_with_a_log_is_taken() {
        let (id, usage) = check_send(&send(ID, log())).unwrap();
        assert_eq!(id, ID);
        assert_eq!(serde_json::from_str::<Value>(&usage).unwrap(), log());
    }

    #[test]
    fn anything_else_is_refused() {
        assert_eq!(check_send(b"not json").unwrap_err().0, 400);
        assert_eq!(check_send(&send("short", log())).unwrap_err().0, 400);
        assert_eq!(
            check_send(&send(&ID.to_uppercase(), log())).unwrap_err().0,
            400
        );
        assert_eq!(check_send(&send(ID, json!({}))).unwrap_err().0, 400);
        let extra = json!({ "send_id": ID, "usage": log(), "who": "someone" });
        assert_eq!(check_send(extra.to_string().as_bytes()).unwrap_err().0, 400);
        let mut big = log();
        big["days"]["2026-11-27"]["calls"]["X"] = json!("x".repeat(MAX_SEND));
        assert_eq!(check_send(&send(ID, big)).unwrap_err().0, 413);
    }

    #[test]
    fn a_log_with_errors_and_pages_is_taken() {
        let mut full = log();
        full["days"]["2026-11-26"] = json!({
            "calls": { "ListSources": 3, "GetUsage": 1 },
            "pages": { "library": 2, "settings-about": 1 },
            "errors": { "RemoveSource": { "NotFound": 1 } },
        });
        full["app_version"] = json!("2.6.0-nightly.20261127+abc");
        assert!(check_send(&send(ID, full)).is_ok());
        let mut empty = log();
        empty["days"] = json!({});
        assert!(check_send(&send(ID, empty)).is_ok());
    }

    #[test]
    fn a_log_of_the_wrong_shape_is_refused() {
        let refused = |change: &dyn Fn(&mut Value)| {
            let mut usage = log();
            change(&mut usage);
            check_send(&send(ID, usage)).unwrap_err().0
        };
        fn day(usage: &mut Value) -> &mut Value {
            &mut usage["days"]["2026-11-27"]
        }
        // The format, the header and the top level
        assert_eq!(refused(&|u| u["format"] = json!(2)), 400);
        assert_eq!(refused(&|u| u["format"] = json!("1")), 400);
        assert_eq!(
            refused(&|u| u["os"] = json!("x".repeat(MAX_HEADER + 1))),
            400
        );
        assert_eq!(refused(&|u| u["os"] = json!("/srv/someone")), 400);
        assert_eq!(refused(&|u| u["platform"] = json!("")), 400);
        assert_eq!(refused(&|u| u["app_version"] = json!(2)), 400);
        assert_eq!(refused(&|u| u["install_id"] = json!("abc")), 400);
        assert_eq!(refused(&|u| u["days"] = json!([])), 400);
        // Day keys are dates
        for key in [
            "2026-13-01",
            "2026-11-32",
            "26-11-27",
            "2026/11/27",
            "today",
            "",
        ] {
            assert_eq!(
                refused(&|u| u["days"] = json!({ key: { "calls": { "Scan": 1 } } })),
                400,
                "{key}"
            );
        }
        let many: serde_json::Map<String, Value> = (0..=MAX_DAYS)
            .map(|i| {
                (
                    format!("2026-{:02}-{:02}", i / 28 + 1, i % 28 + 1),
                    json!({}),
                )
            })
            .collect();
        assert_eq!(refused(&|u| u["days"] = Value::Object(many.clone())), 400);
        // A day holds count maps under short keys, and nothing else
        assert_eq!(
            refused(&|u| day(u)["pages"] = json!({ "/srv/media/Some Show": 1 })),
            400
        );
        assert_eq!(
            refused(&|u| day(u)["pages"] = json!({ "someone@example.com": 1 })),
            400
        );
        assert_eq!(
            refused(&|u| day(u)["calls"] = json!({ "x".repeat(MAX_KEY + 1): 1 })),
            400
        );
        assert_eq!(
            refused(&|u| day(u)["calls"] = json!({ "Scan": "two" })),
            400
        );
        assert_eq!(refused(&|u| day(u)["calls"] = json!({ "Scan": -1 })), 400);
        assert_eq!(refused(&|u| day(u)["calls"] = json!({ "Scan": 1.5 })), 400);
        assert_eq!(refused(&|u| day(u)["notes"] = json!({ "a": 1 })), 400);
        assert_eq!(refused(&|u| day(u)["errors"] = json!({ "Scan": 1 })), 400);
        assert_eq!(
            refused(&|u| day(u)["errors"] = json!({ "Scan": { "C:\\Users": 1 } })),
            400
        );
        assert_eq!(refused(&|u| u["days"]["2026-11-27"] = json!(5)), 400);
    }
}
