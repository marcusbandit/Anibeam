//! The developer endpoints at `dev.anibeam.app`, a Cloudflare Worker on the free plan.
//! Endpoints only, no pages.
//!
//! - `POST /usage` takes a usage log an app's owner chose to send (#1532). It is the one
//!   route anyone may call.
//! - `GET /usage` reads the sends, `DELETE /usage` prunes them.
//!
//! Every other route, and every other method on a public path, is the owner's: it passes
//! [`auth::check`], which wants `Authorization: Bearer <DEV_READ_TOKEN>`. A path this
//! Worker does not know answers `404` before the token is looked at.
//!
//! A new endpoint is one module with its own `route` function, one entry in [`OWNER`],
//! and one arm in [`owner_route`]. See `docs/agents/dev-endpoint.md`.

mod auth;
mod usage;

pub use usage::UsageObject;

use std::net::{IpAddr, Ipv6Addr};

use worker::{Context, Date, Env, Method, Request, Response, Result, event};

/// Every path the owner's routes answer on. A path not listed is `404`.
const OWNER: &[&str] = &["/usage"];

fn now_ms() -> u64 {
    Date::now().as_millis()
}

fn text(status: u16, body: &str) -> Result<Response> {
    Ok(Response::ok(body)?.with_status(status))
}

/// Whether a request is one anyone may make: an app posting a usage log.
fn public(method: &Method, path: &str) -> bool {
    *method == Method::Post && path == "/usage"
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let path = req.path();
    let method = req.method();
    if public(&method, &path) {
        return usage::receive(req, &env).await;
    }
    if !OWNER.contains(&path.as_str()) {
        return text(404, "not found");
    }
    if let Some(refused) = auth::check(&req, &env)? {
        return Ok(refused);
    }
    owner_route(req, &env, &path).await
}

/// The owner's routes, reached only past [`auth::check`].
async fn owner_route(req: Request, env: &Env, path: &str) -> Result<Response> {
    match path {
        "/usage" => usage::route(req, env).await,
        _ => text(404, "not found"),
    }
}

/// The key an address is counted under: an IPv6 address by its /64, so one machine
/// cannot step around the limit by changing its last bits.
fn limit_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let s = v6.segments();
            IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
        v4 => v4,
    }
}

/// Counts one request from the caller's address against `binding`. A missing binding or
/// a failed check lets the request through. The address is counted, never stored.
async fn allowed(env: &Env, binding: &str, req: &Request) -> bool {
    let Ok(limiter) = env.rate_limiter(binding) else {
        return true;
    };
    let ip = req
        .headers()
        .get("CF-Connecting-IP")
        .ok()
        .flatten()
        .unwrap_or_default();
    let key = match ip.parse::<IpAddr>() {
        Ok(ip) => limit_key(ip).to_string(),
        Err(_) => ip,
    };
    limiter
        .limit(key)
        .await
        .map(|outcome| outcome.success)
        .unwrap_or(true)
}

/// A whole number from the query, `Ok(None)` when it is not there, `Err` when it is
/// there and not a number.
fn query_number(url: &worker::Url, key: &str) -> std::result::Result<Option<i64>, ()> {
    match url.query_pairs().find(|(k, _)| k == key) {
        None => Ok(None),
        Some((_, v)) => v.parse::<i64>().map(Some).map_err(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_posting_a_usage_log_is_public() {
        assert!(public(&Method::Post, "/usage"));
        assert!(!public(&Method::Get, "/usage"));
        assert!(!public(&Method::Delete, "/usage"));
        assert!(!public(&Method::Put, "/usage"));
        assert!(!public(&Method::Post, "/usage/"));
        assert!(!public(&Method::Post, "/other"));
    }

    #[test]
    fn an_ipv6_address_counts_by_its_64() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:bbbb::2".parse().unwrap();
        let c: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(limit_key(a), limit_key(b));
        assert_ne!(limit_key(a), limit_key(c));
        let mapped: IpAddr = "::ffff:198.51.100.7".parse().unwrap();
        assert_eq!(limit_key(mapped), "198.51.100.7".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn query_numbers_are_whole_or_refused() {
        let url = |q: &str| worker::Url::parse(&format!("https://dev.example/usage{q}")).unwrap();
        assert_eq!(query_number(&url(""), "since"), Ok(None));
        assert_eq!(
            query_number(&url("?since=1764201600000"), "since"),
            Ok(Some(1764201600000))
        );
        assert_eq!(query_number(&url("?since=yesterday"), "since"), Err(()));
        assert_eq!(query_number(&url("?after=7&since=3"), "after"), Ok(Some(7)));
    }
}
