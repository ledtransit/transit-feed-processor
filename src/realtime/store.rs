use anyhow::{Context, Result};
use log::debug;
use sqlx::PgPool;

use super::apply::{StopUpdateRow, TripUpdateRow};
use crate::db::copy;

/// Pre-rendered `COPY` bodies for one `(feed, scope, rt_source)` snapshot.
pub struct Snapshot {
    pub feed: String,
    pub scope: String,
    pub rt_source: String,
    pub trips: String,
    pub stops: String,
    /// Whether `stops` is a complete, from-scratch picture of every stop the
    /// source has an opinion on for each trip in `trips` this poll (true for
    /// a dense gtfs-rt feed, which re-sends the trip's whole StopTimeUpdate
    /// list every time), vs. only ever a handful of stops per trip by design
    /// (e.g. a source that only reports the stops around the vehicle's
    /// current position, no matter how long the trip is). Controls whether [`write`] may
    /// infer "not in this poll's stop batch" as "no longer part of the
    /// trip" (see `PRUNE_STALE_STOPS`) — for a source that's never dense,
    /// that inference is simply false every single poll, and would delete
    /// every stop reading the moment the source's live window moves past it.
    pub full_stop_coverage: bool,
}

pub fn serialize(
    feed: &str,
    scope: &str,
    rt_source: &str,
    trips: &[TripUpdateRow],
    stops: &[StopUpdateRow],
    full_stop_coverage: bool,
) -> Snapshot {
    let trips_body = copy::body(trips, |w, t| {
        w.text(feed);
        w.text(scope);
        w.text(rt_source);
        w.text(&t.trip_id);
        w.date(t.service_date);
        w.text(&t.route_id);
        w.num(t.start_time);
        w.num(t.end_time);
        w.num(t.schedule_relationship);
        w.bool(t.has_realtime);
        w.opt_num(t.trip_delay);
        w.opt_num(t.rt_timestamp);
        w.opt_text(t.vehicle_id.as_deref());
        w.opt_text(t.vehicle_label.as_deref());
    });
    let stops_body = copy::body(stops, |w, s| {
        w.text(feed);
        w.text(scope);
        w.text(rt_source);
        w.text(&s.trip_id);
        w.date(s.service_date);
        w.num(s.stop_sequence);
        w.text(&s.stop_id);
        w.num(s.schedule_relationship);
        w.bool(s.reported);
        w.opt_num(s.arrival_delay);
        w.opt_num(s.departure_delay);
        w.opt_num(s.arrival_time);
        w.opt_num(s.departure_time);
    });

    Snapshot {
        feed: feed.to_owned(),
        scope: scope.to_owned(),
        rt_source: rt_source.to_owned(),
        trips: trips_body,
        stops: stops_body,
        full_stop_coverage,
    }
}

/// Drop realtime rows for service dates older than yesterday. This is the
/// only place `rt_trip_update` rows are ever deleted outright — [`write`]
/// never removes a row for a trip that's merely absent from a poll's
/// snapshot, so this is what eventually reclaims them, along with rows from
/// sources removed from the registry and gaps left by downtime.
/// (`rt_stop_time_update` follows via `on delete cascade`.)
pub async fn prune_stale(pool: &PgPool) -> Result<()> {
    let deleted = sqlx::query("delete from rt_trip_update where service_date < current_date - 1")
        .execute(pool)
        .await
        .context("prune stale rt rows")?
        .rows_affected();
    if deleted > 0 {
        debug!("pruned {deleted} stale rt_trip_update rows");
    }
    Ok(())
}

/// Merge the `(feed, scope, rt_source)` snapshot in one transaction: upsert
/// every candidate and keep last-known state for anything this poll doesn't
/// mention, whether the feed went silent about a trip still in its candidate
/// window or the trip simply isn't a candidate this poll. Rows are otherwise
/// immortal here — they only go away via [`prune_stale`], once their service
/// date is in the past, via the sticky `CANCELED`/`DELETED` stop-pruning
/// below (always), or via the "stop no longer in the trip" stop-pruning
/// below (only when `snap.full_stop_coverage` says this source's stop batch
/// is actually dense enough for that inference to be trustworthy).
pub async fn write(pool: &PgPool, snap: &Snapshot) -> Result<()> {
    debug!(
        "[{}/{}] merging rt snapshot: {} trip bytes, {} stop bytes",
        snap.rt_source,
        snap.scope,
        snap.trips.len(),
        snap.stops.len()
    );

    let mut tx = pool.begin().await?;
    sqlx::query("set local synchronous_commit = off")
        .execute(&mut *tx)
        .await
        .context("set synchronous_commit")?;

    // Stage the fresh snapshot (unindexed, dropped at commit).
    sqlx::query(
        "create temp table _rt_trip (like rt_trip_update including defaults) on commit drop",
    )
    .execute(&mut *tx)
    .await
    .context("create _rt_trip")?;
    sqlx::query(
        "create temp table _rt_stop (like rt_stop_time_update including defaults) on commit drop",
    )
    .execute(&mut *tx)
    .await
    .context("create _rt_stop")?;

    copy::send(
        &mut tx,
        "copy _rt_trip (feed, scope, rt_source, trip_id, service_date, route_id, start_time, \
         end_time, schedule_relationship, has_realtime, trip_delay, rt_timestamp, vehicle_id, \
         vehicle_label) from stdin",
        &snap.trips,
    )
    .await?;
    copy::send(
        &mut tx,
        "copy _rt_stop (feed, scope, rt_source, trip_id, service_date, stop_sequence, stop_id, \
         schedule_relationship, reported, arrival_delay, departure_delay, arrival_time, \
         departure_time) from stdin",
        &snap.stops,
    )
    .await?;

    sqlx::query(UPSERT_TRIPS)
        .execute(&mut *tx)
        .await
        .context("upsert rt_trip_update")?;

    sqlx::query(UPSERT_STOPS)
        .execute(&mut *tx)
        .await
        .context("upsert rt_stop_time_update")?;

    sqlx::query(PRUNE_CANCELED_STOPS)
        .bind(&snap.feed)
        .bind(&snap.scope)
        .bind(&snap.rt_source)
        .execute(&mut *tx)
        .await
        .context("prune canceled rt_stop_time_update")?;
    if snap.full_stop_coverage {
        sqlx::query(PRUNE_STALE_STOPS)
            .bind(&snap.feed)
            .bind(&snap.scope)
            .bind(&snap.rt_source)
            .execute(&mut *tx)
            .await
            .context("prune stale rt_stop_time_update")?;
    }

    tx.commit().await.context("commit")?;
    Ok(())
}

const UPSERT_TRIPS: &str = r#"
insert into rt_trip_update as t (
    feed, scope, rt_source, trip_id, service_date, route_id, start_time, end_time,
    schedule_relationship, has_realtime, trip_delay, rt_timestamp,
    vehicle_id, vehicle_label, updated_at, rt_updated_at
)
select
    feed, scope, rt_source, trip_id, service_date, route_id, start_time, end_time,
    schedule_relationship, has_realtime, trip_delay, rt_timestamp,
    vehicle_id, vehicle_label,
    now(),
    case when has_realtime then now() end
from _rt_trip
on conflict (feed, scope, rt_source, trip_id, service_date) do update set
    route_id   = excluded.route_id,
    start_time = excluded.start_time,
    end_time   = excluded.end_time,
    updated_at = now(),
    -- a confident poll's schedule_relationship always wins outright, canceled
    -- included — there's no source where a fresh, confident claim (whatever
    -- it says) is less trustworthy than what we already had. Only a silent
    -- poll (no fresh has_realtime data) falls back to the last known value.
    schedule_relationship = case
        when excluded.has_realtime then excluded.schedule_relationship
        else t.schedule_relationship end,
    has_realtime  = t.has_realtime or excluded.has_realtime,
    trip_delay    = case when excluded.has_realtime then excluded.trip_delay    else t.trip_delay    end,
    rt_timestamp  = case when excluded.has_realtime then excluded.rt_timestamp  else t.rt_timestamp  end,
    vehicle_id    = case when excluded.has_realtime then excluded.vehicle_id    else t.vehicle_id    end,
    vehicle_label = case when excluded.has_realtime then excluded.vehicle_label else t.vehicle_label end,
    rt_updated_at = case when excluded.has_realtime then now() else t.rt_updated_at end
"#;

const UPSERT_STOPS: &str = r#"
insert into rt_stop_time_update as s (
    feed, scope, rt_source, trip_id, service_date, stop_sequence, stop_id,
    schedule_relationship, reported, arrival_delay, departure_delay,
    arrival_time, departure_time, updated_at, rt_updated_at
)
select
    feed, scope, rt_source, trip_id, service_date, stop_sequence, stop_id,
    schedule_relationship, reported, arrival_delay, departure_delay,
    arrival_time, departure_time, now(), now()
from _rt_stop
on conflict (feed, scope, rt_source, trip_id, service_date, stop_sequence) do update set
    stop_id       = excluded.stop_id,
    updated_at    = now(),
    rt_updated_at = now(),
    -- only a stop the source reported may change status: one it left out (a
    -- partial update) keeps e.g. a skip, while a reported one can revise it
    schedule_relationship = case
        when excluded.reported then excluded.schedule_relationship
        else s.schedule_relationship end,
    reported      = excluded.reported,
    arrival_delay   = excluded.arrival_delay,
    departure_delay = excluded.departure_delay,
    arrival_time    = excluded.arrival_time,
    departure_time  = excluded.departure_time
"#;

// Drop stop rows of a trip that's now canceled/deleted. Always runs,
// regardless of `full_stop_coverage` — a canceled trip's old stop readings
// are meaningless (the run they described didn't happen) no matter how
// sparse the source that reported them is.
const PRUNE_CANCELED_STOPS: &str = r#"
delete from rt_stop_time_update s
using rt_trip_update t
where s.feed = $1 and s.scope = $2 and s.rt_source = $3
  and t.feed = s.feed and t.scope = s.scope and t.rt_source = s.rt_source
  and t.trip_id = s.trip_id and t.service_date = s.service_date
  and t.schedule_relationship in (3, 7)
"#;

// Drop a stop row an actively-updated trip no longer materialises this poll
// (e.g. the static schedule changed). Only run when `snap.full_stop_coverage`
// — this poll's stop batch has to actually be a complete, from-scratch
// picture of the trip's stops for "missing from it" to mean "no longer part
// of the trip" rather than just "this source never covers every stop every
// poll" (for a source that only reports the few stops around the vehicle's
// current position, this query would otherwise delete a stop's whole
// reading the moment its live window moves past it).
const PRUNE_STALE_STOPS: &str = r#"
delete from rt_stop_time_update s
using rt_trip_update t
where s.feed = $1 and s.scope = $2 and s.rt_source = $3
  and t.feed = s.feed and t.scope = s.scope and t.rt_source = s.rt_source
  and t.trip_id = s.trip_id and t.service_date = s.service_date
  and exists (select 1 from _rt_trip i
              where i.trip_id = s.trip_id and i.service_date = s.service_date
                and i.has_realtime)
  and not exists (select 1 from _rt_stop j
                  where j.trip_id = s.trip_id and j.service_date = s.service_date
                    and j.stop_sequence = s.stop_sequence)
"#;
