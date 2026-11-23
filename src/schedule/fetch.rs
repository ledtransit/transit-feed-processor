use anyhow::{Context, Result};
use log::debug;
use reqwest::StatusCode;
use reqwest::header::IF_NONE_MATCH;

use crate::feed::RemoteSource;
use crate::net;

use super::cache::{FeedDirs, Validator};

pub enum Fetched {
    Unchanged,
    Skipped, // Credentials unavailable
    Downloaded,
}

/// Download the static zip straight to disk, next to the current one, and
/// only move it in place once complete and actually new: the zip is never
/// held in memory, and a failed or unchanged download leaves the previous
/// zip untouched.
pub async fn fetch(src: &RemoteSource, dirs: &FeedDirs) -> Result<Fetched> {
    let prior = dirs.read_validator()?;

    let Some(mut req) = net::get(src)? else {
        return Ok(Fetched::Skipped);
    };
    match &prior {
        Some(Validator::Etag(tag)) => {
            debug!("conditional GET {} (If-None-Match: {tag})", src.url);
            req = req.header(IF_NONE_MATCH, tag);
        }
        _ => debug!("GET {}", src.url),
    }

    let resp = match net::download_retrying(req, src, &dirs.part).await {
        Ok(resp) => resp,
        Err(e) => {
            discard(dirs);
            return Err(e);
        }
    };
    if resp.status == StatusCode::NOT_MODIFIED {
        debug!("304 Not Modified");
        return Ok(Fetched::Unchanged);
    }
    let hash = resp.sha256;
    debug!("downloaded {} bytes (sha256 {}…)", resp.bytes, &hash[..12]);

    if resp.etag.is_none()
        && let Some(Validator::Hash(prev)) = &prior
        && *prev == hash
    {
        debug!("content hash unchanged, skipping");
        discard(dirs);
        return Ok(Fetched::Unchanged);
    }

    tokio::fs::rename(&dirs.part, &dirs.zip)
        .await
        .with_context(|| format!("move download to {}", dirs.zip.display()))?;
    let validator = match resp.etag {
        Some(tag) => Validator::Etag(tag),
        None => Validator::Hash(hash),
    };
    debug!("stored zip and {validator:?}");
    dirs.write_validator(&validator)?;

    Ok(Fetched::Downloaded)
}

/// Drop a partial or unneeded download, if any.
fn discard(dirs: &FeedDirs) {
    let _ = std::fs::remove_file(&dirs.part);
}
