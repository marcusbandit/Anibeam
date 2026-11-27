//! The usage log's collection point (#1532). An app sends its usage log here only when its
//! owner presses send: `POST /usage` with `{"send_id", "usage"}`, a fresh random id per send
//! and the log's counts. Each send is kept as one row in one SQLite-backed Durable Object
//! (`UsageObject`), size capped and rate limited per address before the object is touched.
//! `GET /usage` reads them back, only with `Authorization: Bearer <USAGE_READ_TOKEN>`, a
//! Worker secret. Nothing here records the sender's address: the limiter counts it and
//! forgets it.

use serde::Deserialize;
use serde_json::{Value, json};
use worker::{
    DurableObject, Env, Method, Request, RequestInit, Response, Result, State, durable_object,
};

use crate::{allowed, now_ms, text};

/// The Durable Object namespace binding in `wrangler.toml`.
const USAGE: &str = "USAGE";

/// Sends per address a minute, set in `wrangler.toml`.
const USAGE_LIMIT: &str = "USAGE_LIMIT";

/// The secret `GET /usage` checks.
const READ_TOKEN: &str = "USAGE_READ_TOKEN";

/// The largest send.
pub const MAX_SEND: usize = 256 * 1024;

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

/// The send's id and its log as compact JSON, or why the body is not a send: valid JSON,
/// a 32 character hex id, and a log with a format, the header strings and a days object.
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
    let short = |key: &str| {
        send.usage
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty() && s.len() <= 64)
    };
    let log_ok = send.usage.get("format").and_then(Value::as_u64).is_some()
        && short("app_version")
        && short("platform")
        && short("os")
        && send.usage.get("days").is_some_and(Value::is_object);
    if !log_ok {
        return Err((400, "usage must be a usage log"));
    }
    Ok((send.send_id, send.usage.to_string()))
}

/// Whether the request carries the read token. Compared over every byte, so the time it
/// takes says nothing about how much of a guess was right.
pub fn token_ok(header: Option<&str>, token: &str) -> bool {
    let Some(given) = header.and_then(|h| h.strip_prefix("Bearer ")) else {
        return false;
    };
    if token.is_empty() || given.len() != token.len() {
        return false;
    }
    given
        .bytes()
        .zip(token.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// `POST /usage` and `GET /usage`.
pub async fn route(req: Request, env: &Env) -> Result<Response> {
    match req.method() {
        Method::Post => receive(req, env).await,
        Method::Get => read(req, env).await,
        _ => text(405, "method not allowed"),
    }
}

async fn receive(mut req: Request, env: &Env) -> Result<Response> {
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
    let token = env.secret(READ_TOKEN).map(|s| s.to_string()).ok();
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return text(404, "not found");
    };
    let header = req.headers().get("Authorization").ok().flatten();
    if !token_ok(header.as_deref(), &token) {
        return text(401, "unauthorized");
    }
    let after = req
        .url()?
        .query_pairs()
        .find(|(k, _)| k == "after")
        .and_then(|(_, v)| v.parse::<i64>().ok())
        .unwrap_or(0);
    let internal = Request::new(&format!("{INTERNAL}/list?after={after}"), Method::Get)?;
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
        match (req.method(), path.as_str()) {
            (Method::Post, "/store") => {
                #[derive(Deserialize)]
                struct Store {
                    send_id: String,
                    usage: String,
                }
                let row: Store = req.json().await?;
                let cursor = self.state.storage().sql().exec(
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
                text(201, "stored")
            }
            (Method::Get, "/list") => {
                let after = req
                    .url()?
                    .query_pairs()
                    .find(|(k, _)| k == "after")
                    .and_then(|(_, v)| v.parse::<i64>().ok())
                    .unwrap_or(0);
                let rows: Vec<Row> = self
                    .state
                    .storage()
                    .sql()
                    .exec(
                        "SELECT id, at, send_id, body FROM sends WHERE id > ? ORDER BY id LIMIT ?",
                        vec![after.into(), PAGE.into()],
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
    fn only_the_exact_token_reads() {
        assert!(token_ok(Some("Bearer s3cret"), "s3cret"));
        assert!(!token_ok(Some("Bearer s3cre"), "s3cret"));
        assert!(!token_ok(Some("Bearer s3creT"), "s3cret"));
        assert!(!token_ok(Some("s3cret"), "s3cret"));
        assert!(!token_ok(None, "s3cret"));
        assert!(!token_ok(Some("Bearer "), ""));
    }
}
