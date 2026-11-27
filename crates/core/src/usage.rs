//! The usage log's calls (#1532). The counting and the file are
//! `anibeam_foundation::usage`; the core counts every call it routes and every page the
//! shell says it opened, and answers the calls that show, save, clear and send the log.
//! `SendUsage` is the only code that sends it anywhere: once, to the collection route on
//! the rooms relay, with a fresh random id that is never kept, so two sends from one
//! machine cannot be tied together.
//!
//! A count never touches the disk on the thread that made it: the save it asks for runs on
//! the core's runtime, at most once a minute (`save_later`). Showing, saving, sending and
//! clearing the log, and shutdown, save at once.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::contract::*;
use crate::core::Core;
use crate::foundation::blocking::blocking;
use crate::net::{Body, HttpRequest};

pub use anibeam_foundation::usage::UsageLog as Log;

/// Where a send goes: the collection route on the rooms relay, never a relay the person
/// picked for rooms, which may be their own and have no such route.
pub fn send_url() -> String {
    format!("{ROOM_RELAY}/usage")
}

pub(crate) fn note(core: &Core, page: &str) -> Result<Reply, CoreError> {
    if let Some(after) = core.usage.record_page(page)? {
        save_later(core, after);
    }
    Ok(Reply::Ok)
}

/// The save a count asked for, `after` from now, on a blocking thread of the core's
/// runtime. A runtime already shut down drops it; shutdown saved first.
pub(crate) fn save_later(core: &Core, after: Duration) {
    let log = Arc::clone(&core.usage);
    core.services.handle.spawn(async move {
        tokio::time::sleep(after).await;
        let _ = tokio::task::spawn_blocking(move || log.save_due()).await;
    });
}

pub(crate) fn get(core: &Core) -> Result<Reply, CoreError> {
    let text = core.usage.text();
    let (days, events) = core.usage.summary();
    Ok(Reply::Usage {
        usage: UsageLog {
            text,
            days,
            events,
            enabled: core.usage.enabled(),
        },
    })
}

/// The text a preview showed, checked to be a usage log, or the log as it is now.
fn text_or_now(core: &Core, text: Option<String>) -> Result<String, CoreError> {
    match text {
        Some(text) => Log::checked(&text).map_err(|e| {
            CoreError::invalid(
                "text",
                crate::i18n::tr("core-usage-not-a-log", &[("error", e.into())]),
            )
        }),
        None => Ok(core.usage.text()),
    }
}

pub(crate) fn export(core: &Core, path: &str, text: Option<String>) -> Result<Reply, CoreError> {
    let text = text_or_now(core, text)?;
    crate::atomic_file::write(std::path::Path::new(path), text)?;
    Ok(Reply::Ok)
}

pub(crate) fn clear(core: &Core) -> Result<Reply, CoreError> {
    core.usage.clear()?;
    Ok(Reply::Ok)
}

/// A random id for one send: 128 bits as hex.
fn send_id() -> String {
    rand::random::<[u8; 16]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Sends the log the preview showed, or as `GetUsage` shows it now, under a fresh send id. Waits on the network, so a
/// shell sends it off its UI thread.
pub(crate) fn send(core: &Core, text: Option<String>) -> Result<Reply, CoreError> {
    let usage: serde_json::Value = serde_json::from_str(&text_or_now(core, text)?)
        .map_err(|e| CoreError::internal(format!("usage log: {e}")))?;
    let body = json!({ "send_id": send_id(), "usage": usage });
    let req = HttpRequest::post(send_url()).body(Body::Json(body));
    let services = &core.services;
    let res = blocking(|| services.handle.block_on(services.http.send(req))).map_err(|e| {
        CoreError::service(
            "usage",
            "unreachable",
            crate::i18n::tr("core-usage-unreachable", &[("error", e.message.into())]),
        )
    })?;
    if !res.is_success() {
        return Err(CoreError::service(
            "usage",
            format!("http_{}", res.status),
            crate::i18n::tr(
                "core-usage-refused",
                &[("status", i64::from(res.status).into())],
            ),
        ));
    }
    Ok(Reply::Ok)
}
