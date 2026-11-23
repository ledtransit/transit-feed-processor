use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use log::debug;
use reqwest::StatusCode;
use reqwest::header::IF_NONE_MATCH;

use crate::feed::RemoteSource;
use crate::net;

const BASE: &str = "data/gtfs-rt";

pub enum Fetched {
    Unchanged,
    Skipped, // Credentials unavailable
    Body(Vec<u8>),
}

/// Download one gtfs-rt feed into `data/gtfs-rt/<ident>/feed.pb`. When the
/// server sent an `ETag` on the previous fetch, revalidate with `If-None-Match`
/// and skip on `304`.
pub async fn fetch(src: &RemoteSource, ident: &str) -> Result<Fetched> {
    let dirs = RtDirs::new(ident);
    dirs.ensure()?;

    let prior_etag = dirs.read_etag();
    let Some(mut req) = net::get(src)? else {
        return Ok(Fetched::Skipped);
    };
    match &prior_etag {
        Some(tag) => {
            debug!(
                "[{ident}] conditional GET {} (If-None-Match: {tag})",
                src.url
            );
            req = req.header(IF_NONE_MATCH, tag);
        }
        None => debug!("[{ident}] GET {}", src.url),
    }

    let resp = net::fetch_retrying(req, src).await?;
    if resp.status == StatusCode::NOT_MODIFIED {
        debug!("[{ident}] 304 Not Modified");
        return Ok(Fetched::Unchanged);
    }

    let body = resp.body;
    debug!("[{ident}] downloaded {} bytes", body.len());

    fs::write(&dirs.pb, &body).with_context(|| format!("write {}", dirs.pb.display()))?;
    match resp.etag {
        Some(tag) => dirs.write_etag(&tag)?,
        None => dirs.clear_etag(),
    }

    Ok(Fetched::Body(body))
}

struct RtDirs {
    root: PathBuf,
    pb: PathBuf,
    etag: PathBuf,
}

impl RtDirs {
    fn new(ident: &str) -> Self {
        let root = Path::new(BASE).join(ident);
        Self {
            pb: root.join("feed.pb"),
            etag: root.join("feed.etag"),
            root,
        }
    }

    fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.root).with_context(|| format!("create {}", self.root.display()))
    }

    fn read_etag(&self) -> Option<String> {
        fs::read_to_string(&self.etag)
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    }

    fn write_etag(&self, tag: &str) -> Result<()> {
        fs::write(&self.etag, tag).with_context(|| format!("write {}", self.etag.display()))
    }

    fn clear_etag(&self) {
        let _ = fs::remove_file(&self.etag);
    }
}
