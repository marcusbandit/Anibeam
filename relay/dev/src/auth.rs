//! The one check every owner route passes: `Authorization: Bearer <DEV_READ_TOKEN>`,
//! where `DEV_READ_TOKEN` is a Worker secret that is never in the repo.

use worker::{Env, Request, Response, Result};

/// The secret the owner's routes check.
pub const TOKEN: &str = "DEV_READ_TOKEN";

/// Why a request is refused, as the status and the text the caller gets.
pub type Refusal = (u16, &'static str);

/// Whether `header` carries exactly `token`. Compared over every byte, so the time it
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

/// What the check answers for this header and this secret: `None` lets the request in.
/// A Worker without the secret refuses everyone, so a deploy that forgot it is never
/// open.
pub fn refusal(header: Option<&str>, secret: Option<&str>) -> Option<Refusal> {
    match secret.filter(|s| !s.is_empty()) {
        None => Some((503, "DEV_READ_TOKEN is not set on this Worker")),
        Some(token) if token_ok(header, token) => None,
        Some(_) => Some((401, "unauthorized")),
    }
}

/// The check, on a live request: `Ok(None)` lets it in, `Ok(Some(response))` is the
/// refusal to send back.
pub fn check(req: &Request, env: &Env) -> Result<Option<Response>> {
    let secret = env
        .secret(TOKEN)
        .map(|s| s.to_string())
        .or_else(|_| env.var(TOKEN).map(|v| v.to_string()))
        .ok();
    let header = req.headers().get("Authorization").ok().flatten();
    let Some((status, why)) = refusal(header.as_deref(), secret.as_deref()) else {
        return Ok(None);
    };
    let mut res = Response::ok(why)?.with_status(status);
    if status == 401 {
        res.headers_mut().set("WWW-Authenticate", "Bearer")?;
    }
    Ok(Some(res))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_token_reads() {
        assert!(token_ok(Some("Bearer s3cret"), "s3cret"));
        assert!(!token_ok(Some("Bearer s3cre"), "s3cret"));
        assert!(!token_ok(Some("Bearer s3creT"), "s3cret"));
        assert!(!token_ok(Some("s3cret"), "s3cret"));
        assert!(!token_ok(Some("bearer s3cret"), "s3cret"));
        assert!(!token_ok(None, "s3cret"));
        assert!(!token_ok(Some("Bearer "), ""));
    }

    #[test]
    fn a_worker_without_the_secret_lets_nobody_in() {
        assert_eq!(refusal(Some("Bearer "), None).unwrap().0, 503);
        assert_eq!(refusal(Some("Bearer "), Some("")).unwrap().0, 503);
        assert_eq!(refusal(None, None).unwrap().0, 503);
    }

    #[test]
    fn the_right_token_passes_and_any_other_is_401() {
        assert_eq!(refusal(Some("Bearer s3cret"), Some("s3cret")), None);
        assert_eq!(refusal(Some("Bearer nope"), Some("s3cret")).unwrap().0, 401);
        assert_eq!(refusal(None, Some("s3cret")).unwrap().0, 401);
    }
}
