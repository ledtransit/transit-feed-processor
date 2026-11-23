use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use sqlx::PgPool;

use crate::feed::GtfsFeedScope;
use crate::timing::Timer;

/// One scope's candidates, computed once per feed poll and shared by every
/// realtime source of that feed (each gtfs-rt entry, plus any plugin source).
#[derive(Clone)]
pub struct ScopeCandidates {
    pub scope: &'static GtfsFeedScope,
    pub trips: Arc<Vec<CandidateTrip>>,
}

/// Every scope's candidates for one feed, in scope-declaration order.
pub type FeedCandidates = Vec<ScopeCandidates>;

/// Compute [`running_now`] for every scope of a feed, once. The result is
/// wrapped in `Arc` so it can be handed to several concurrent realtime
/// sources without cloning the trip data.
pub async fn for_feed(
    pool: &PgPool,
    feed: &str,
    scopes: &'static [GtfsFeedScope],
    tz: Tz,
    now: DateTime<Utc>,
) -> Result<FeedCandidates> {
    let mut out = Vec::with_capacity(scopes.len());
    for scope in scopes {
        let trips = {
            let _t = Timer::start(format!("[{feed}/{}] candidates", scope.ident));
            running_now(
                pool,
                feed,
                scope.ident,
                tz,
                now,
                scope.max_early,
                scope.max_late,
            )
            .await?
        };
        out.push(ScopeCandidates {
            scope,
            trips: Arc::new(trips),
        });
    }
    Ok(out)
}

/// One trip run (a `trip_id` on a specific service date) that could plausibly
/// be in progress right now, before any realtime adjustment. Enriched with the
/// route/agency/direction fields and full ordered stop schedule needed by
/// every realtime source (gtfs-rt and any plugin matcher) — computed once per
/// `(feed, scope)` and shared, so no source re-queries it.
// route_short_name/agency_name/direction_id/trip_headsign and the stop
// name/coordinates below are read by fuzzy realtime matchers in the plugin
// (`--features plugin`), not by the gtfs-rt apply path — hence the blanket
// dead_code allowance in a default build.
#[derive(Debug)]
#[cfg_attr(not(feature = "plugin"), allow(dead_code))]
pub struct CandidateTrip {
    pub trip_id: String,
    pub route_id: String,
    pub route_short_name: Option<String>,
    pub agency_name: Option<String>,
    #[allow(dead_code)] // not read by any matcher yet
    pub direction_id: Option<i16>,
    pub trip_headsign: Option<String>,
    pub service_date: NaiveDate,
    /// Scheduled span, seconds after `service_date` midnight (may exceed 86400).
    pub start_time: i32,
    pub end_time: i32,
    /// Full scheduled stop sequence, ordered by `stop_sequence`.
    pub stops: Vec<CandidateStop>,
}

/// One scheduled stop of a candidate trip, seconds after service-date midnight.
#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "plugin"), allow(dead_code))]
pub struct CandidateStop {
    pub stop_sequence: i32,
    pub stop_id: String,
    pub stop_name: Option<String>,
    pub stop_lat: Option<f64>,
    pub stop_lon: Option<f64>,
    pub arrival_time: Option<i32>,
    pub departure_time: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct TripRow {
    trip_id: String,
    route_id: String,
    route_short_name: Option<String>,
    agency_name: Option<String>,
    direction_id: Option<i16>,
    trip_headsign: Option<String>,
    service_date: NaiveDate,
    start_time: i32,
    end_time: i32,
}

#[derive(sqlx::FromRow)]
struct StopRow {
    trip_id: String,
    stop_sequence: i32,
    stop_id: String,
    stop_name: Option<String>,
    stop_lat: Option<f64>,
    stop_lon: Option<f64>,
    arrival_time: Option<i32>,
    departure_time: Option<i32>,
}

const SECONDS_PER_DAY: i32 = 86_400;

/// Trips from the static mirror whose scheduled span could overlap `now`,
/// each carrying its full stop schedule. Two queries total regardless of how
/// many realtime sources end up using the result.
///
/// A vehicle may run up to `max_early` ahead of or `max_late` behind schedule,
/// so we look for trips whose `[start_time, end_time]` intersects
/// `[now - max_late, now + max_early]`:
/// - back by `max_late`  — a trip scheduled to end that long ago may still be running,
/// - forward by `max_early` — a trip scheduled to start that soon may already be running early.
///
/// GTFS times are seconds since (noon − 12h) of the *service date* and can
/// exceed 24h, so a run active now may belong to yesterday's, today's or
/// tomorrow's service date. All three are evaluated, each in its own seconds
/// frame; calendar + calendar_dates resolution happens in SQL.
pub async fn running_now(
    pool: &PgPool,
    feed: &str,
    scope: &str,
    tz: Tz,
    now: DateTime<Utc>,
    max_early: Duration,
    max_late: Duration,
) -> Result<Vec<CandidateTrip>> {
    let local = now.with_timezone(&tz);
    let today = local.date_naive();
    let now_secs = local.num_seconds_from_midnight() as i32;
    let early = max_early.as_secs() as i32;
    let late = max_late.as_secs() as i32;

    let service_dates = vec![
        today.pred_opt().context("service date underflow")?,
        today,
        today.succ_opt().context("service date overflow")?,
    ];
    // `now` in each service date's own seconds frame: yesterday is +1 day, etc.
    let offsets = [SECONDS_PER_DAY, 0, -SECONDS_PER_DAY];
    let lo: Vec<i32> = offsets.iter().map(|off| now_secs + off - late).collect();
    let hi: Vec<i32> = offsets.iter().map(|off| now_secs + off + early).collect();

    let trips: Vec<TripRow> = sqlx::query_as(TRIP_QUERY)
        .bind(feed)
        .bind(scope)
        .bind(&service_dates)
        .bind(&lo)
        .bind(&hi)
        .fetch_all(pool)
        .await
        .context("query running trips")?;

    let trip_ids: Vec<&str> = trips.iter().map(|t| t.trip_id.as_str()).collect();
    let mut stops_by_trip = stops_for(pool, feed, scope, &trip_ids).await?;

    Ok(trips
        .into_iter()
        .map(|t| CandidateTrip {
            stops: stops_by_trip.remove(&t.trip_id).unwrap_or_default(),
            trip_id: t.trip_id,
            route_id: t.route_id,
            route_short_name: t.route_short_name,
            agency_name: t.agency_name,
            direction_id: t.direction_id,
            trip_headsign: t.trip_headsign,
            service_date: t.service_date,
            start_time: t.start_time,
            end_time: t.end_time,
        })
        .collect())
}

/// Scheduled stops (joined with `stops` for name/coordinates) for the given
/// trips, grouped by `trip_id` and ordered by `stop_sequence` within each.
async fn stops_for(
    pool: &PgPool,
    feed: &str,
    scope: &str,
    trip_ids: &[&str],
) -> Result<HashMap<String, Vec<CandidateStop>>> {
    if trip_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<StopRow> = sqlx::query_as(
        "select st.trip_id, st.stop_sequence, st.stop_id, \
                s.stop_name, s.stop_lat, s.stop_lon, \
                st.arrival_time, st.departure_time \
         from stop_times st \
         join stops s on s.feed = st.feed and s.scope = st.scope and s.stop_id = st.stop_id \
         where st.feed = $1 and st.scope = $2 and st.trip_id = any($3) \
         order by st.trip_id, st.stop_sequence",
    )
    .bind(feed)
    .bind(scope)
    .bind(trip_ids)
    .fetch_all(pool)
    .await
    .context("query candidate stop_times")?;

    let mut by_trip: HashMap<String, Vec<CandidateStop>> = HashMap::new();
    for r in rows {
        by_trip.entry(r.trip_id).or_default().push(CandidateStop {
            stop_sequence: r.stop_sequence,
            stop_id: r.stop_id,
            stop_name: r.stop_name,
            stop_lat: r.stop_lat,
            stop_lon: r.stop_lon,
            arrival_time: r.arrival_time,
            departure_time: r.departure_time,
        });
    }
    Ok(by_trip)
}

// `route_short_name`/`agency_name` are best-effort context for fuzzy realtime
// matching in the plugin, not relied on for gtfs-rt correctness — an unmatched
// agency_id (only valid when the scope has exactly one agency; see
// schedule::filter) simply leaves `agency_name` null.
const TRIP_QUERY: &str = r#"
with win as (
    select service_date, lo, hi
    from unnest($3::date[], $4::int[], $5::int[]) as w(service_date, lo, hi)
)
select
    t.trip_id, t.route_id, w.service_date, t.start_time, t.end_time,
    t.direction_id, t.trip_headsign,
    coalesce(nullif(r.route_short_name, ''), r.route_long_name) as route_short_name,
    a.agency_name
from win w
join trips t
  on  t.feed = $1
  and t.scope = $2
  and t.start_time is not null
  and int4range(t.start_time, t.end_time, '[]') && int4range(w.lo, w.hi, '[]')
left join routes r on r.feed = t.feed and r.scope = t.scope and r.route_id = t.route_id
left join agencies a on a.feed = r.feed and a.scope = r.scope and a.agency_id = r.agency_id
where
    exists (
        select 1 from calendar_dates cd
        where cd.feed = $1 and cd.scope = $2 and cd.service_id = t.service_id
          and cd.date = w.service_date and cd.exception_type = 1
    )
    or (
        exists (
            select 1 from calendar c
            where c.feed = $1 and c.scope = $2 and c.service_id = t.service_id
              and w.service_date between c.start_date and c.end_date
              and case extract(isodow from w.service_date)::int
                    when 1 then c.monday    when 2 then c.tuesday
                    when 3 then c.wednesday when 4 then c.thursday
                    when 5 then c.friday    when 6 then c.saturday
                    when 7 then c.sunday
                  end
        )
        and not exists (
            select 1 from calendar_dates cd
            where cd.feed = $1 and cd.scope = $2 and cd.service_id = t.service_id
              and cd.date = w.service_date and cd.exception_type = 2
        )
    )
"#;
