use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use log::debug;
use sqlx::PgPool;

use super::filter::{ScopeSink, Selected};
use super::gtfs::{
    AgencyRow, CalendarDateRow, CalendarRow, RouteRow, StopRow, StopTimeRow, TripRow,
};
use crate::db::copy;
use crate::timing::Timer;

/// Whether the mirror already has any routes for this feed, in any scope.
pub async fn has_data(pool: &PgPool, feed: &str) -> Result<bool> {
    sqlx::query_scalar("select exists(select 1 from routes where feed = $1)")
        .bind(feed)
        .fetch_one(pool)
        .await
        .context("check existing mirror data")
}

/// `COPY ... FROM STDIN` bodies for every table, pre-rendered off the async
/// runtime (see [`serialize`]).
pub struct CopyBundle {
    pub feed: String,
    pub scope: String,
    pub agencies: String,
    pub routes: String,
    pub stops: String,
    pub calendar: String,
    pub calendar_dates: String,
    pub trips: String,
    pub stop_times: String,
}

/// Render every table to `COPY` text. Consumes `data` (see [`CopyWriter`],
/// which renders rows as they come instead of all at the end).
pub fn serialize(feed: &str, scope: &str, data: Selected) -> Result<CopyBundle> {
    let mut w = CopyWriter::new(feed, scope);
    w.agencies(data.agencies);
    w.routes(data.routes);
    data.trips.into_iter().for_each(|r| w.trip(r));
    data.stop_times.into_iter().for_each(|r| w.stop_time(r));
    data.stops.into_iter().for_each(|r| w.stop(r));
    data.calendar.into_iter().for_each(|r| w.calendar(r));
    data.calendar_dates
        .into_iter()
        .for_each(|r| w.calendar_date(r));
    w.finish()
}

/// Row counts of what a [`CopyWriter`] rendered.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub routes: usize,
    pub trips: usize,
    pub stop_times: usize,
    pub stops: usize,
    pub calendar: usize,
    pub calendar_dates: usize,
}

/// A scope's selection rendered straight to `COPY` text as its rows arrive,
/// so they needn't all be held: only trips wait for [`finish`](Self::finish),
/// since their span comes from the stop_times that follow them.
pub struct CopyWriter {
    feed: String,
    scope: String,
    agencies: String,
    routes: String,
    stops: String,
    calendar: String,
    calendar_dates: String,
    stop_times: String,
    trips: Vec<TripRow>,
    /// `trip_id -> (min start, max end)`, see [`extend_span`].
    spans: HashMap<String, (i32, i32)>,
    counts: Counts,
    /// The first row that couldn't be rendered.
    error: Option<anyhow::Error>,
}

impl CopyWriter {
    pub fn new(feed: &str, scope: &str) -> Self {
        Self {
            feed: feed.to_owned(),
            scope: scope.to_owned(),
            agencies: String::new(),
            routes: String::new(),
            stops: String::new(),
            calendar: String::new(),
            calendar_dates: String::new(),
            stop_times: String::new(),
            trips: Vec::new(),
            spans: HashMap::new(),
            counts: Counts::default(),
            error: None,
        }
    }

    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// The rendered bodies, or the first row that couldn't be rendered.
    pub fn finish(self) -> Result<CopyBundle> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let (feed, scope, spans) = (&self.feed, &self.scope, &self.spans);
        let trips = copy::body(&self.trips, |w, t| {
            let (start, end) = match spans.get(t.trip_id.as_str()) {
                Some(&(s, e)) => (Some(s), Some(e)),
                None => (None, None),
            };
            w.text(feed);
            w.text(scope);
            w.text(&t.trip_id);
            w.text(&t.route_id);
            w.text(&t.service_id);
            w.opt_text(t.trip_headsign.as_deref());
            w.opt_text(t.trip_short_name.as_deref());
            w.opt_num(t.direction_id);
            w.opt_text(t.block_id.as_deref());
            w.opt_text(t.shape_id.as_deref());
            w.opt_num(t.wheelchair_accessible);
            w.opt_num(t.bikes_allowed);
            w.opt_num(start);
            w.opt_num(end);
        });
        Ok(CopyBundle {
            feed: self.feed,
            scope: self.scope,
            agencies: self.agencies,
            routes: self.routes,
            stops: self.stops,
            calendar: self.calendar,
            calendar_dates: self.calendar_dates,
            trips,
            stop_times: self.stop_times,
        })
    }

    fn fail(&mut self, e: anyhow::Error) {
        self.error.get_or_insert(e);
    }
}

impl ScopeSink for CopyWriter {
    fn agencies(&mut self, rows: Vec<AgencyRow>) {
        let (feed, scope) = (&self.feed, &self.scope);
        self.agencies = copy::body(&rows, |w, a| {
            w.text(feed);
            w.text(scope);
            w.text(&a.agency_id);
            w.text(&a.agency_name);
            w.text(&a.agency_timezone);
        });
    }

    fn routes(&mut self, rows: Vec<RouteRow>) {
        let (feed, scope) = (&self.feed, &self.scope);
        self.counts.routes = rows.len();
        self.routes = copy::body(&rows, |w, r| {
            w.text(feed);
            w.text(scope);
            w.text(&r.route_id);
            w.opt_text(r.agency_id.as_deref());
            w.opt_text(r.route_short_name.as_deref());
            w.opt_text(r.route_long_name.as_deref());
            w.opt_text(r.route_desc.as_deref());
            w.num(r.route_type);
            w.opt_text(r.route_url.as_deref());
            w.text(nonempty(&r.route_color).unwrap_or("FFFFFF"));
            w.text(nonempty(&r.route_text_color).unwrap_or("000000"));
            w.opt_num(r.route_sort_order);
            w.opt_num(r.continuous_pickup);
            w.opt_num(r.continuous_drop_off);
            w.opt_text(r.network_id.as_deref());
        });
    }

    fn trip(&mut self, row: TripRow) {
        self.counts.trips += 1;
        self.trips.push(row);
    }

    fn stop_time(&mut self, st: StopTimeRow) {
        self.counts.stop_times += 1;
        let (arrival, departure) = (seconds(&st.arrival_time), seconds(&st.departure_time));
        extend_span(&mut self.spans, &st.trip_id, arrival, departure);
        let (feed, scope) = (&self.feed, &self.scope);
        copy::line(&mut self.stop_times, |w| {
            w.text(feed);
            w.text(scope);
            w.text(&st.trip_id);
            w.num(st.stop_sequence);
            w.text(&st.stop_id);
            w.opt_num(arrival);
            w.opt_num(departure);
            w.opt_text(st.stop_headsign.as_deref());
            w.opt_num(st.pickup_type);
            w.opt_num(st.drop_off_type);
            w.opt_num(st.continuous_pickup);
            w.opt_num(st.continuous_drop_off);
            w.opt_num(st.shape_dist_traveled);
            w.opt_num(st.timepoint);
        });
    }

    fn stop(&mut self, s: StopRow) {
        self.counts.stops += 1;
        let (feed, scope) = (&self.feed, &self.scope);
        copy::line(&mut self.stops, |w| {
            w.text(feed);
            w.text(scope);
            w.text(&s.stop_id);
            w.opt_text(s.stop_code.as_deref());
            w.opt_text(s.stop_name.as_deref());
            w.opt_text(s.tts_stop_name.as_deref());
            w.opt_text(s.stop_desc.as_deref());
            w.opt_num(s.stop_lat);
            w.opt_num(s.stop_lon);
            w.opt_text(s.zone_id.as_deref());
            w.opt_text(s.stop_url.as_deref());
            w.opt_num(s.location_type);
            w.opt_text(s.parent_station.as_deref());
            w.opt_text(s.stop_timezone.as_deref());
            w.opt_num(s.wheelchair_boarding);
            w.opt_text(s.level_id.as_deref());
            w.opt_text(s.platform_code.as_deref());
        });
    }

    fn calendar(&mut self, r: CalendarRow) {
        let dates = parse_date(&r.start_date).and_then(|s| Ok((s, parse_date(&r.end_date)?)));
        let (start, end) = match dates {
            Ok(dates) => dates,
            Err(e) => return self.fail(e),
        };
        self.counts.calendar += 1;
        let days = [
            r.monday,
            r.tuesday,
            r.wednesday,
            r.thursday,
            r.friday,
            r.saturday,
            r.sunday,
        ];
        let (feed, scope) = (&self.feed, &self.scope);
        copy::line(&mut self.calendar, |w| {
            w.text(feed);
            w.text(scope);
            w.text(&r.service_id);
            for d in days {
                w.bool(d == 1);
            }
            w.date(start);
            w.date(end);
        });
    }

    fn calendar_date(&mut self, r: CalendarDateRow) {
        let date = match parse_date(&r.date) {
            Ok(date) => date,
            Err(e) => return self.fail(e),
        };
        self.counts.calendar_dates += 1;
        let (feed, scope) = (&self.feed, &self.scope);
        copy::line(&mut self.calendar_dates, |w| {
            w.text(feed);
            w.text(scope);
            w.text(&r.service_id);
            w.date(date);
            w.num(r.exception_type);
        });
    }
}

pub async fn write(pool: &PgPool, bundle: &CopyBundle) -> Result<()> {
    let feed = &bundle.feed;
    let scope = &bundle.scope;
    debug!(
        "[{feed}/{scope}] copying {} agencies / {} routes / {} stops / {} calendar / {} calendar_dates / {} trips / {} stop_times bytes",
        bundle.agencies.len(),
        bundle.routes.len(),
        bundle.stops.len(),
        bundle.calendar.len(),
        bundle.calendar_dates.len(),
        bundle.trips.len(),
        bundle.stop_times.len(),
    );

    let mut tx = pool.begin().await?;

    // Bulk-load tuning, scoped to this transaction:
    //  - `replica` skips FK-trigger revalidation; the filter cascade already
    //    guarantees referential integrity, and children are deleted first below.
    //  - `synchronous_commit = off`; the mirror is rebuilt from source on loss.
    for stmt in [
        "set local session_replication_role = replica",
        "set local synchronous_commit = off",
    ] {
        sqlx::query(stmt)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("running {stmt}"))?;
    }

    {
        let _t = Timer::start(format!("[{feed}/{scope}] db delete"));
        for stmt in [
            "delete from stop_times where feed = $1 and scope = $2",
            "delete from trips where feed = $1 and scope = $2",
            "delete from calendar_dates where feed = $1 and scope = $2",
            "delete from calendar where feed = $1 and scope = $2",
            "delete from stops where feed = $1 and scope = $2",
            "delete from routes where feed = $1 and scope = $2",
            "delete from agencies where feed = $1 and scope = $2",
        ] {
            sqlx::query(stmt)
                .bind(feed)
                .bind(scope)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("running {stmt}"))?;
        }
    }

    {
        let _t = Timer::start(format!("[{feed}/{scope}] db copy"));
        copy::send(
            &mut tx,
            "copy agencies (feed, scope, agency_id, agency_name, agency_timezone) from stdin",
            &bundle.agencies,
        )
        .await?;
        copy::send(
            &mut tx,
            "copy routes (feed, scope, route_id, agency_id, route_short_name, route_long_name, \
             route_desc, route_type, route_url, route_color, route_text_color, route_sort_order, \
             continuous_pickup, continuous_drop_off, network_id) from stdin",
            &bundle.routes,
        )
        .await?;
        copy::send(
            &mut tx,
            "copy stops (feed, scope, stop_id, stop_code, stop_name, tts_stop_name, stop_desc, \
             stop_lat, stop_lon, zone_id, stop_url, location_type, parent_station, stop_timezone, \
             wheelchair_boarding, level_id, platform_code) from stdin",
            &bundle.stops,
        )
        .await?;
        copy::send(
            &mut tx,
            "copy calendar (feed, scope, service_id, monday, tuesday, wednesday, thursday, friday, \
             saturday, sunday, start_date, end_date) from stdin",
            &bundle.calendar,
        )
        .await?;
        copy::send(
            &mut tx,
            "copy calendar_dates (feed, scope, service_id, date, exception_type) from stdin",
            &bundle.calendar_dates,
        )
        .await?;
        copy::send(
            &mut tx,
            "copy trips (feed, scope, trip_id, route_id, service_id, trip_headsign, trip_short_name, \
             direction_id, block_id, shape_id, wheelchair_accessible, bikes_allowed, start_time, \
             end_time) from stdin",
            &bundle.trips,
        )
        .await?;
        copy::send(
            &mut tx,
            "copy stop_times (feed, scope, trip_id, stop_sequence, stop_id, arrival_time, \
             departure_time, stop_headsign, pickup_type, drop_off_type, continuous_pickup, \
             continuous_drop_off, shape_dist_traveled, timepoint) from stdin",
            &bundle.stop_times,
        )
        .await?;
    }

    tx.commit().await.context("commit")?;

    // Keep the realtime lookup's planner honest after a full reload.
    let _ = sqlx::query("analyze trips, stop_times").execute(pool).await;

    Ok(())
}

/// Widen `trip_id`'s span in `spans` — `(min start, max end)` in seconds
/// after service-date midnight — by one of its stop_times, preferring
/// `departure` for the start and `arrival` for the end.
fn extend_span(
    spans: &mut HashMap<String, (i32, i32)>,
    trip_id: &str,
    arrival: Option<i32>,
    departure: Option<i32>,
) {
    let Some(start) = departure.or(arrival) else {
        return;
    };
    let end = arrival.or(departure).unwrap_or(start).max(start);
    match spans.get_mut(trip_id) {
        Some((s, e)) => {
            *s = (*s).min(start);
            *e = (*e).max(end);
        }
        None => {
            spans.insert(trip_id.to_owned(), (start, end));
        }
    }
}

fn nonempty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|v| !v.is_empty())
}

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y%m%d").with_context(|| format!("invalid date {s:?}"))
}

fn seconds(time: &Option<String>) -> Option<i32> {
    let raw = time.as_deref()?;
    let mut parts = raw.trim().split(':');
    let h: i32 = parts.next()?.parse().ok()?;
    let m: i32 = parts.next()?.parse().ok()?;
    let s: i32 = parts.next()?.parse().ok()?;
    Some(h * 3600 + m * 60 + s)
}
