pub mod apply;
pub mod candidates;
mod fetch;
pub mod proto;
pub mod store;

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use log::{debug, error, info};
use sqlx::PgPool;
use tokio::task::JoinSet;

use crate::db;
use crate::feed::{FEEDS, Feed, GtfsRtFeed};
use crate::plugin::{self, FeedProcPlugin};
use crate::timing::Timer;
use candidates::FeedCandidates;
use proto::gtfs_realtime::{FeedMessage, TripUpdate};

pub async fn run_all_realtime(pool: &PgPool) -> Result<()> {
    if let Err(e) = store::prune_stale(pool).await {
        error!("rt prune failed: {e:#}");
    }

    let mut set = JoinSet::new();
    for feed in FEEDS {
        let pool = pool.clone();
        set.spawn(async move {
            if let Err(e) = poll_feed(feed, &pool).await {
                let ident = feed.schedule.as_ref().map_or("?", |s| s.ident);
                error!("[{ident}] realtime poll failed: {e:#}");
            }
        });
    }
    while set.join_next().await.is_some() {}
    Ok(())
}

async fn poll_feed(feed: &'static Feed, pool: &PgPool) -> Result<()> {
    let Some(schedule) = feed.schedule.as_ref() else {
        return Ok(());
    };
    let static_feed = schedule.ident;
    let tz = feed.timezone;
    let now = Utc::now();

    // Get candidate trips possible running now, computed once and shared by every realtime source
    let scoped = candidates::for_feed(pool, static_feed, schedule.scopes, tz, now).await?;

    let mut set = JoinSet::new();
    for rt in feed.realtime {
        let pool = pool.clone();
        let scoped = scoped.clone();
        set.spawn(async move {
            match poll_gtfs_rt(static_feed, tz, rt, &scoped, now, &pool).await {
                Ok(Some(s)) => info!(
                    "[{}] {} candidate trips ({} with realtime), {} stop updates",
                    rt.ident, s.trips, s.with_realtime, s.stops
                ),
                Ok(None) => debug!("[{}] realtime unchanged or skipped", rt.ident),
                Err(e) => error!("[{}] realtime poll failed: {e:#}", rt.ident),
            }
        });
    }
    {
        let pool = pool.clone();
        let scoped = scoped.clone();
        set.spawn(async move { plugin::run_realtime(feed, &scoped, &pool).await });
    }
    while set.join_next().await.is_some() {}

    // Notify per feed once all its rt sources are done, so a slow feed doesn't
    // hold back the others
    let rt_sources: Vec<&str> = feed
        .realtime
        .iter()
        .map(|rt| rt.ident)
        .chain(feed.plugin.as_ref().map(FeedProcPlugin::ident))
        .collect();
    db::notify::notify_realtime_updated(pool, &rt_sources)
        .await
        .context("notify realtime updated")
}

struct PollSummary {
    trips: usize,
    with_realtime: usize,
    stops: usize,
}

async fn poll_gtfs_rt(
    static_feed: &'static str,
    tz: Tz,
    rt: &'static GtfsRtFeed,
    scoped: &FeedCandidates,
    now: DateTime<Utc>,
    pool: &PgPool,
) -> Result<Option<PollSummary>> {
    // Download gtfs-rt protobuf
    let bytes = {
        let _t = Timer::start(format!("[{}] download", rt.ident));
        match fetch::fetch(&rt.source, rt.ident).await? {
            fetch::Fetched::Unchanged | fetch::Fetched::Skipped => return Ok(None),
            fetch::Fetched::Body(bytes) => bytes,
        }
    };

    // Decode gtfs-rt protobuf and extract trip updates
    let trip_updates = {
        let _t = Timer::start(format!("[{}] decode", rt.ident));
        let message: FeedMessage =
            prost::Message::decode(bytes.as_ref()).context("decode gtfs-rt FeedMessage")?;
        let updates = message
            .entity
            .into_iter()
            .filter_map(|entity| entity.trip_update)
            .collect::<Vec<TripUpdate>>();
        Arc::new(updates)
    };

    let mut total = PollSummary {
        trips: 0,
        with_realtime: 0,
        stops: 0,
    };
    for sc in scoped {
        let scope_ident = sc.scope.ident;
        let candidates = sc.trips.clone();
        let trip_updates = trip_updates.clone();
        let (summary, snapshot) = tokio::task::spawn_blocking(move || {
            // Apply the trip updates to the candidate trips, producing a snapshot
            let _t = Timer::start(format!("[{}/{scope_ident}] apply", rt.ident));
            let (trip_rows, stop_rows) =
                apply::apply(tz, candidates.as_ref(), trip_updates.as_ref(), now);
            let summary = PollSummary {
                trips: trip_rows.len(),
                with_realtime: trip_rows.iter().filter(|t| t.has_realtime).count(),
                stops: stop_rows.len(),
            };
            // true: apply::apply forward-fills a full StopUpdateRow per trip
            // stop every poll, so "missing this poll" trustworthily means
            // "no longer in the trip" (see Snapshot::full_stop_coverage).
            (
                summary,
                store::serialize(
                    static_feed,
                    scope_ident,
                    rt.ident,
                    &trip_rows,
                    &stop_rows,
                    true,
                ),
            )
        })
        .await
        .context("apply task panicked")?;

        {
            // Store the snapshot in the database
            let _t = Timer::start(format!("[{}/{scope_ident}] db write", rt.ident));
            store::write(pool, &snapshot).await?;
        }

        debug!(
            "[{}/{scope_ident}] {} candidate trips ({} with realtime), {} stop updates",
            rt.ident, summary.trips, summary.with_realtime, summary.stops
        );
        total.trips += summary.trips;
        total.with_realtime += summary.with_realtime;
        total.stops += summary.stops;
    }

    Ok(Some(total))
}
