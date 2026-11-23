mod cache;
mod fetch;
pub mod filter;
pub mod gtfs;
pub mod store;

use anyhow::{Context, Result};
use log::{debug, error, info};
use sqlx::PgPool;

use crate::{
    feed::{FEEDS, Feed},
    timing::Timer,
};
use cache::FeedDirs;
use fetch::Fetched;

/// Refresh every feed that has a static GTFS source, one after another: each
/// one's selection and COPY bodies are freed before the next is read, so the
/// memory needed is that of the largest feed, not of all of them together.
pub async fn run_all_static(pool: &PgPool) -> Result<()> {
    for feed in FEEDS {
        let Some(schedule) = &feed.schedule else {
            continue;
        };
        let ident = schedule.ident;
        match process(feed, pool).await {
            Ok(summary) => info!("[{ident}] {summary}"),
            Err(e) => error!("[{ident}] static refresh failed: {e:#}"),
        }
    }
    Ok(())
}

async fn process(feed: &'static Feed, pool: &PgPool) -> Result<String> {
    let cfg = feed.schedule.as_ref().expect("caller filtered for Some");
    let ident = cfg.ident;
    debug!("[{ident}] starting static refresh");

    // Make path
    let dirs = FeedDirs::new(ident);
    dirs.ensure()?;

    // Download static zip
    let fetched = {
        let _t = Timer::start(format!("[{ident}] download"));
        fetch::fetch(&cfg.source, &dirs).await?
    };
    if let Fetched::Skipped = fetched {
        return Ok("skipped, credentials unavailable".to_owned());
    }
    if let Fetched::Unchanged = fetched
        && store::has_data(pool, ident).await?
    {
        return Ok("unchanged, skipped".to_owned());
    }
    // New download, or unchanged but missing from the db: (re)load it.

    // Stream the zip's tables, rendering what each scope selects straight
    // to its COPY bodies
    let zip = dirs.zip.clone();
    let scopes = cfg.scopes;
    let timezone = feed.timezone;
    let rendered = tokio::task::spawn_blocking(
        move || -> Result<Vec<(&'static str, store::Counts, store::CopyBundle)>> {
            let _t = Timer::start(format!("[{ident}] parse, filter and serialize"));
            let mut archive = gtfs::Archive::open(&zip)?;
            let filters: Vec<_> = scopes.iter().map(|s| s.filters).collect();
            let mut writers: Vec<_> = scopes
                .iter()
                .map(|s| store::CopyWriter::new(ident, s.ident))
                .collect();
            filter::select_scopes(&mut archive, &filters, timezone, &mut writers)?;
            scopes
                .iter()
                .zip(writers)
                .map(|(s, w)| Ok((s.ident, w.counts(), w.finish()?)))
                .collect()
        },
    )
    .await
    .context("parse task panicked")??;

    let mut summary = String::new();
    for (scope_ident, counts, bundle) in rendered {
        {
            let _t = Timer::start(format!("[{ident}/{scope_ident}] db write"));
            store::write(pool, &bundle).await?;
        }
        if !summary.is_empty() {
            summary.push_str("; ");
        }
        summary.push_str(&format!(
            "[{scope_ident}] stored {} routes, {} trips, {} stop_times, {} stops, {} calendar, {} calendar_dates",
            counts.routes,
            counts.trips,
            counts.stop_times,
            counts.stops,
            counts.calendar,
            counts.calendar_dates,
        ));
    }

    Ok(summary)
}
