//! DB-backed tests for `candidates::running_now` / `TRIP_QUERY`.
//!
//! Each `#[sqlx::test]` gets a fresh database with `./migrations` applied. Needs
//! a reachable Postgres with CREATEDB rights via `DATABASE_URL` (e.g. from
//! `.env`). Run: `cargo test --test realtime candidates_running_now`.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use sqlx::PgPool;
use transit_feed_processor::realtime::candidates::{CandidateTrip, running_now};

const FEED: &str = "vbb";
const SCOPE: &str = "berlin_s_u";

const EVERY_DAY: [bool; 7] = [true; 7];

// Wednesday 2024-06-12, 12:00 Europe/Berlin (10:00Z, CEST +2). now_secs = 43200.
// With max_early = 15min, max_late = 60min the probe windows are:
//   yesterday 2024-06-11 : [126000, 130500]
//   today     2024-06-12 : [ 39600,  44100]
//   tomorrow  2024-06-13 : [-46800, -42300]
fn now_wed_noon() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 6, 12, 10, 0, 0).unwrap()
}

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// The UTC instant at which Europe/Berlin shows the given (unambiguous) wall time.
fn berlin_wall(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
    chrono_tz::Europe::Berlin
        .with_ymd_and_hms(y, m, d, h, mi, 0)
        .single()
        .expect("unambiguous Berlin wall time")
        .with_timezone(&Utc)
}

/// Weekday-only bitmap (`mon..sun`) with a single day set (Monday = 0).
fn only(day: usize) -> [bool; 7] {
    let mut b = [false; 7];
    b[day] = true;
    b
}

async fn candidates(pool: &PgPool, now: DateTime<Utc>) -> Vec<CandidateTrip> {
    let mut out = running_now(
        pool,
        FEED,
        SCOPE,
        chrono_tz::Europe::Berlin,
        now,
        Duration::from_secs(15 * 60),
        Duration::from_secs(60 * 60),
    )
    .await
    .expect("running_now");
    out.sort_by(|a, b| {
        (a.trip_id.as_str(), a.service_date).cmp(&(b.trip_id.as_str(), b.service_date))
    });
    out
}

// ---- seeding ----

async fn add_agency(pool: &PgPool, agency_id: &str, name: &str) {
    sqlx::query(
        "insert into agencies (feed, scope, agency_id, agency_name, agency_timezone) \
         values ($1, $2, $3, $4, 'Europe/Berlin')",
    )
    .bind(FEED)
    .bind(SCOPE)
    .bind(agency_id)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
}

async fn add_route(
    pool: &PgPool,
    route_id: &str,
    agency_id: Option<&str>,
    short: Option<&str>,
    long: Option<&str>,
) {
    sqlx::query(
        "insert into routes (feed, scope, route_id, agency_id, route_short_name, route_long_name, route_type) \
         values ($1, $2, $3, $4, $5, $6, 3)",
    )
    .bind(FEED)
    .bind(SCOPE)
    .bind(route_id)
    .bind(agency_id)
    .bind(short)
    .bind(long)
    .execute(pool)
    .await
    .unwrap();
}

async fn add_trip(
    pool: &PgPool,
    trip_id: &str,
    route_id: &str,
    service_id: &str,
    start: Option<i32>,
    end: Option<i32>,
) {
    sqlx::query(
        "insert into trips (feed, scope, trip_id, route_id, service_id, start_time, end_time) \
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(FEED)
    .bind(SCOPE)
    .bind(trip_id)
    .bind(route_id)
    .bind(service_id)
    .bind(start)
    .bind(end)
    .execute(pool)
    .await
    .unwrap();
}

async fn add_stop(
    pool: &PgPool,
    stop_id: &str,
    name: Option<&str>,
    lat: Option<f64>,
    lon: Option<f64>,
) {
    sqlx::query(
        "insert into stops (feed, scope, stop_id, stop_name, stop_lat, stop_lon) \
         values ($1, $2, $3, $4, $5, $6)",
    )
    .bind(FEED)
    .bind(SCOPE)
    .bind(stop_id)
    .bind(name)
    .bind(lat)
    .bind(lon)
    .execute(pool)
    .await
    .unwrap();
}

async fn add_stop_time(
    pool: &PgPool,
    trip_id: &str,
    seq: i32,
    stop_id: &str,
    arr: Option<i32>,
    dep: Option<i32>,
) {
    sqlx::query(
        "insert into stop_times (feed, scope, trip_id, stop_sequence, stop_id, arrival_time, departure_time) \
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(FEED)
    .bind(SCOPE)
    .bind(trip_id)
    .bind(seq)
    .bind(stop_id)
    .bind(arr)
    .bind(dep)
    .execute(pool)
    .await
    .unwrap();
}

async fn add_calendar(
    pool: &PgPool,
    service_id: &str,
    days: [bool; 7],
    start: NaiveDate,
    end: NaiveDate,
) {
    sqlx::query(
        "insert into calendar \
         (feed, scope, service_id, monday, tuesday, wednesday, thursday, friday, saturday, sunday, start_date, end_date) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(FEED)
    .bind(SCOPE)
    .bind(service_id)
    .bind(days[0])
    .bind(days[1])
    .bind(days[2])
    .bind(days[3])
    .bind(days[4])
    .bind(days[5])
    .bind(days[6])
    .bind(start)
    .bind(end)
    .execute(pool)
    .await
    .unwrap();
}

async fn add_calendar_date(pool: &PgPool, service_id: &str, date: NaiveDate, exception_type: i16) {
    sqlx::query(
        "insert into calendar_dates (feed, scope, service_id, date, exception_type) values ($1, $2, $3, $4, $5)",
    )
    .bind(FEED)
    .bind(SCOPE)
    .bind(service_id)
    .bind(date)
    .bind(exception_type)
    .execute(pool)
    .await
    .unwrap();
}

/// One agency + one route (`R1`, short name `U1`), the base for most tests.
async fn base(pool: &PgPool) {
    add_agency(pool, "A1", "Agency One").await;
    add_route(pool, "R1", Some("A1"), Some("U1"), None).await;
}

fn year() -> (NaiveDate, NaiveDate) {
    (ymd(2024, 1, 1), ymd(2024, 12, 31))
}

// ---- window / range-overlap ----

#[sqlx::test]
async fn trip_running_now_today_is_a_candidate(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "S", EVERY_DAY, year().0, year().1).await;
    add_trip(&pool, "T_NOW", "R1", "S", Some(43_000), Some(43_800)).await;

    let got = candidates(&pool, now_wed_noon()).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].trip_id, "T_NOW");
    assert_eq!(got[0].service_date, ymd(2024, 6, 12));
    assert_eq!(got[0].route_short_name.as_deref(), Some("U1"));
    assert_eq!(got[0].agency_name.as_deref(), Some("Agency One"));
    assert_eq!((got[0].start_time, got[0].end_time), (43_000, 43_800));
}

#[sqlx::test]
async fn trip_outside_every_window_is_excluded(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "S", EVERY_DAY, year().0, year().1).await;
    add_trip(&pool, "T_NIGHT", "R1", "S", Some(3_600), Some(7_200)).await;

    assert!(candidates(&pool, now_wed_noon()).await.is_empty());
}

#[sqlx::test]
async fn after_midnight_trip_belongs_to_previous_service_date(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "S", EVERY_DAY, year().0, year().1).await;
    // ~35.3h into its service date: only the yesterday window [126000, 130500] hits.
    add_trip(&pool, "T_OWL", "R1", "S", Some(127_000), Some(128_000)).await;

    let got = candidates(&pool, now_wed_noon()).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].service_date, ymd(2024, 6, 11));
}

#[sqlx::test]
async fn trip_active_on_two_service_dates_yields_two_candidates(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "S", EVERY_DAY, year().0, year().1).await;
    // Spans today's window and reaches back into yesterday's frame.
    add_trip(&pool, "T_LONG", "R1", "S", Some(40_000), Some(130_000)).await;

    let got = candidates(&pool, now_wed_noon()).await;
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].service_date, ymd(2024, 6, 11));
    assert_eq!(got[1].service_date, ymd(2024, 6, 12));
    assert!(got.iter().all(|c| c.trip_id == "T_LONG"));
    assert!(
        got.iter()
            .all(|c| (c.start_time, c.end_time) == (40_000, 130_000))
    );
}

#[sqlx::test]
async fn trip_with_null_start_time_is_excluded(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "S", EVERY_DAY, year().0, year().1).await;
    add_trip(&pool, "T_NULL", "R1", "S", None, None).await;

    assert!(candidates(&pool, now_wed_noon()).await.is_empty());
}

// ---- calendar / calendar_dates resolution ----

#[sqlx::test]
async fn calendar_weekday_gate_excludes_trip(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "SAT", only(5), year().0, year().1).await; // Saturdays only
    add_trip(&pool, "T", "R1", "SAT", Some(43_000), Some(43_800)).await;

    assert!(candidates(&pool, now_wed_noon()).await.is_empty());
}

#[sqlx::test]
async fn calendar_date_range_excludes_trip(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "PAST", EVERY_DAY, ymd(2024, 1, 1), ymd(2024, 5, 1)).await;
    add_trip(&pool, "T", "R1", "PAST", Some(43_000), Some(43_800)).await;

    assert!(candidates(&pool, now_wed_noon()).await.is_empty());
}

#[sqlx::test]
async fn calendar_dates_removal_excludes_trip(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "S", EVERY_DAY, year().0, year().1).await;
    add_calendar_date(&pool, "S", ymd(2024, 6, 12), 2).await; // removed today
    add_trip(&pool, "T", "R1", "S", Some(43_000), Some(43_800)).await;

    assert!(candidates(&pool, now_wed_noon()).await.is_empty());
}

#[sqlx::test]
async fn calendar_dates_addition_includes_trip_without_calendar_row(pool: PgPool) {
    base(&pool).await;
    add_calendar_date(&pool, "EXC", ymd(2024, 6, 12), 1).await; // added today, no calendar row
    add_trip(&pool, "T_ADDED", "R1", "EXC", Some(43_000), Some(43_800)).await;

    let got = candidates(&pool, now_wed_noon()).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].trip_id, "T_ADDED");
    assert_eq!(got[0].service_date, ymd(2024, 6, 12));
}

#[sqlx::test]
async fn calendar_dates_addition_on_another_date_does_not_apply(pool: PgPool) {
    base(&pool).await;
    add_calendar_date(&pool, "EXC", ymd(2024, 6, 20), 1).await; // added, but not today
    add_trip(&pool, "T", "R1", "EXC", Some(43_000), Some(43_800)).await;

    assert!(candidates(&pool, now_wed_noon()).await.is_empty());
}

// ---- joins: route_short_name / agency_name ----

#[sqlx::test]
async fn route_short_name_falls_back_to_long_name(pool: PgPool) {
    add_agency(&pool, "A1", "Agency One").await;
    add_route(&pool, "R_BLANK", Some("A1"), Some(""), Some("S-Bahn 1")).await;
    add_route(&pool, "R_SHORT", Some("A1"), Some("U2"), Some("U-Bahn 2")).await;
    add_calendar(&pool, "S", EVERY_DAY, year().0, year().1).await;
    add_trip(&pool, "T_BLANK", "R_BLANK", "S", Some(43_000), Some(43_800)).await;
    add_trip(&pool, "T_SHORT", "R_SHORT", "S", Some(43_000), Some(43_800)).await;

    let got = candidates(&pool, now_wed_noon()).await;
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].route_short_name.as_deref(), Some("S-Bahn 1")); // T_BLANK
    assert_eq!(got[1].route_short_name.as_deref(), Some("U2")); // T_SHORT
}

#[sqlx::test]
async fn unknown_or_missing_agency_leaves_agency_name_null(pool: PgPool) {
    add_agency(&pool, "A1", "Agency One").await;
    add_route(&pool, "R_GHOST", Some("A_MISSING"), Some("X1"), None).await;
    add_route(&pool, "R_NOAG", None, Some("X2"), None).await;
    add_calendar(&pool, "S", EVERY_DAY, year().0, year().1).await;
    add_trip(&pool, "T_GHOST", "R_GHOST", "S", Some(43_000), Some(43_800)).await;
    add_trip(&pool, "T_NOAG", "R_NOAG", "S", Some(43_000), Some(43_800)).await;

    let got = candidates(&pool, now_wed_noon()).await;
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|c| c.agency_name.is_none()));
}

// ---- isolation ----

#[sqlx::test]
async fn other_scope_trips_are_not_returned(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "S", EVERY_DAY, year().0, year().1).await;
    add_trip(&pool, "T_MINE", "R1", "S", Some(43_000), Some(43_800)).await;

    // A fully matching trip under a different scope.
    sqlx::query(
        "insert into routes (feed, scope, route_id, route_type) values ($1, 'other', 'R1', 3)",
    )
    .bind(FEED)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into calendar \
         (feed, scope, service_id, monday, tuesday, wednesday, thursday, friday, saturday, sunday, start_date, end_date) \
         values ($1, 'other', 'S', true, true, true, true, true, true, true, '2024-01-01', '2024-12-31')",
    )
    .bind(FEED)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into trips (feed, scope, trip_id, route_id, service_id, start_time, end_time) \
         values ($1, 'other', 'T_OTHER', 'R1', 'S', 43000, 43800)",
    )
    .bind(FEED)
    .execute(&pool)
    .await
    .unwrap();

    let got = candidates(&pool, now_wed_noon()).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].trip_id, "T_MINE");
}

// ---- stops attached to each candidate ----

#[sqlx::test]
async fn stops_are_attached_in_sequence_order(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "S", EVERY_DAY, year().0, year().1).await;
    add_trip(&pool, "T_STOPS", "R1", "S", Some(43_000), Some(43_800)).await;
    add_stop(&pool, "S1", Some("Alpha"), Some(52.5), Some(13.4)).await;
    add_stop(&pool, "S2", Some("Beta"), None, None).await;
    add_stop(&pool, "S3", Some("Gamma"), Some(52.6), Some(13.5)).await;
    // Inserted out of order to prove the query orders by stop_sequence.
    add_stop_time(&pool, "T_STOPS", 3, "S3", Some(43_700), None).await;
    add_stop_time(&pool, "T_STOPS", 1, "S1", None, Some(43_000)).await;
    add_stop_time(&pool, "T_STOPS", 2, "S2", Some(43_300), Some(43_320)).await;

    let got = candidates(&pool, now_wed_noon()).await;
    assert_eq!(got.len(), 1);
    let stops = &got[0].stops;
    assert_eq!(stops.len(), 3);
    assert_eq!(
        stops.iter().map(|s| s.stop_sequence).collect::<Vec<_>>(),
        [1, 2, 3],
    );
    assert_eq!(stops[0].stop_id, "S1");
    assert_eq!(stops[0].stop_name.as_deref(), Some("Alpha"));
    assert_eq!(stops[0].departure_time, Some(43_000));
    assert_eq!(stops[0].arrival_time, None);
    assert_eq!(stops[1].stop_lat, None);
    assert_eq!(stops[2].stop_name.as_deref(), Some("Gamma"));
    assert_eq!(stops[2].stop_lat, Some(52.6));
}

#[sqlx::test]
async fn candidate_without_stop_times_has_no_stops(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "S", EVERY_DAY, year().0, year().1).await;
    add_trip(&pool, "T_BARE", "R1", "S", Some(43_000), Some(43_800)).await;

    let got = candidates(&pool, now_wed_noon()).await;
    assert_eq!(got.len(), 1);
    assert!(got[0].stops.is_empty());
}

// ---- DST transitions ----
//
// GTFS times are seconds from "noon minus 12h" of the service day (midnight on
// normal days, 23:00/01:00 wall time on days a DST switch happens). `running_now`
// approximates that origin with `num_seconds_from_midnight()` (local wall clock)
// and shifts into neighbouring service dates by a flat ±86400. Europe/Berlin
// switches at 01:00 UTC, i.e. before local noon, so once the switch is complete
// the two agree again; the tests below pin the behaviour on both sides.
//
// Berlin 2024: spring-forward Sun 2024-03-31 (02:00->03:00), fall-back
// Sun 2024-10-27 (03:00->02:00).

const SUNDAY: usize = 6;
const SATURDAY: usize = 5;

#[sqlx::test]
async fn fall_back_day_after_transition_finds_running_trip(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "SUN", only(SUNDAY), year().0, year().1).await;
    // Poll 10:00 wall time, hours after the fold: wall clock and GTFS frame agree.
    add_trip(&pool, "T", "R1", "SUN", Some(35_400), Some(36_600)).await;

    let got = candidates(&pool, berlin_wall(2024, 10, 27, 10, 0)).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].trip_id, "T");
    assert_eq!(got[0].service_date, ymd(2024, 10, 27));
}

#[sqlx::test]
async fn spring_forward_day_after_transition_finds_running_trip(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "SUN", only(SUNDAY), year().0, year().1).await;
    add_trip(&pool, "T", "R1", "SUN", Some(35_400), Some(36_600)).await;

    let got = candidates(&pool, berlin_wall(2024, 3, 31, 10, 0)).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].service_date, ymd(2024, 3, 31));
}

#[sqlx::test]
async fn fall_back_day_25th_hour_trip_reachable_via_previous_service_date(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "SUN", only(SUNDAY), year().0, year().1).await;
    // 24:30-25:00 on the fall-back service date (the day has a real 25th hour).
    add_trip(&pool, "T_LATE", "R1", "SUN", Some(88_200), Some(90_000)).await;

    // Poll 00:45 wall time the next calendar day; "now" in the 2024-10-27 frame
    // is 24:45 = now_secs (2700) + 86400 = 89100, inside [88200, 90000].
    let got = candidates(&pool, berlin_wall(2024, 10, 28, 0, 45)).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].trip_id, "T_LATE");
    assert_eq!(got[0].service_date, ymd(2024, 10, 27));
}

#[sqlx::test]
async fn after_midnight_trip_before_spring_forward_uses_previous_service_date(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "SAT", only(SATURDAY), year().0, year().1).await;
    // 24:10-24:30 on Sat 2024-03-30 (a normal day), i.e. wall 00:10-00:30 Sun.
    add_trip(&pool, "T_OWL", "R1", "SAT", Some(87_000), Some(88_200)).await;

    // Poll 00:15 wall time on the spring-forward day, before the 02:00 gap.
    let got = candidates(&pool, berlin_wall(2024, 3, 31, 0, 15)).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].trip_id, "T_OWL");
    assert_eq!(got[0].service_date, ymd(2024, 3, 30));
}

/// Known limitation: within the DST transition window `running_now` frames "now"
/// off the local wall clock, not GTFS's "noon minus 12h" origin. On the
/// spring-forward morning that origin is 23:00 the previous day, so wall 01:00 is
/// GTFS 02:00 — the probe window is an hour behind. A feed that encodes an
/// early trip per spec (GTFS 02:00) is missed until the switch completes, while
/// a wall-clock-naive encoding (GTFS 01:00) is what gets matched. Feeds that keep
/// times in the recommended 03:00-27:00 band never hit this.
#[sqlx::test]
async fn dst_transition_window_frames_now_off_the_wall_clock(pool: PgPool) {
    base(&pool).await;
    add_calendar(&pool, "SUN", only(SUNDAY), year().0, year().1).await;
    // Same physical trip, two encodings for wall 01:00-01:30 on 2024-03-31:
    add_trip(&pool, "T_SPEC", "R1", "SUN", Some(7_200), Some(9_000)).await; // GTFS per noon-12h
    add_trip(&pool, "T_WALL", "R1", "SUN", Some(3_600), Some(5_400)).await; // wall seconds

    // now_secs = 3600; today window is [0, 4500].
    let got = candidates(&pool, berlin_wall(2024, 3, 31, 1, 0)).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].trip_id, "T_WALL");
}
