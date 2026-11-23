//! DB-backed tests for `realtime::store::write` — the per-poll merge of a
//! `(feed, scope, rt_source)` snapshot into `rt_trip_update` /
//! `rt_stop_time_update`: upsert every candidate, and leave anything this
//! poll doesn't mention completely untouched (it's not deleted just for
//! being absent from a snapshot — only `prune_stale` ever does that).
//!
//! `rt_trip_update` has no FK to the static tables, so nothing needs seeding.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::common::*;

const FEED: &str = "vbb";
const SCOPE: &str = "berlin_s_u";
const SRC: &str = "vbb-rt";
const SRC2: &str = "vbb-alt";

async fn put(pool: &PgPool, src: &str, trips: &[TripUpdateRow], stops: &[StopUpdateRow]) {
    write(pool, &serialize(FEED, SCOPE, src, trips, stops, true))
        .await
        .expect("write");
}

/// Like [`put`], but for a source whose stop batch is never a complete
/// picture of a trip's stops — `full_stop_coverage: false`.
async fn put_sparse(pool: &PgPool, src: &str, trips: &[TripUpdateRow], stops: &[StopUpdateRow]) {
    write(pool, &serialize(FEED, SCOPE, src, trips, stops, false))
        .await
        .expect("write");
}

fn stop(trip_id: &str, seq: i32) -> StopUpdateRow {
    stop_update_row(trip_id, seq, &format!("s{seq}"))
}

#[derive(sqlx::FromRow, Debug)]
struct RtTrip {
    rt_source: String,
    trip_id: String,
    schedule_relationship: i16,
    has_realtime: bool,
    trip_delay: Option<i32>,
    rt_timestamp: Option<i64>,
    vehicle_id: Option<String>,
    vehicle_label: Option<String>,
    updated_at: DateTime<Utc>,
    rt_updated_at: Option<DateTime<Utc>>,
}

async fn rt_trips(pool: &PgPool) -> Vec<RtTrip> {
    sqlx::query_as(
        "select rt_source, trip_id, schedule_relationship, has_realtime, trip_delay, rt_timestamp, \
                vehicle_id, vehicle_label, updated_at, rt_updated_at \
         from rt_trip_update order by rt_source, trip_id, service_date",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[derive(sqlx::FromRow, Debug)]
struct RtStop {
    rt_source: String,
    stop_sequence: i32,
    schedule_relationship: i16,
    arrival_delay: Option<i32>,
    departure_delay: Option<i32>,
}

async fn rt_stops(pool: &PgPool) -> Vec<RtStop> {
    sqlx::query_as(
        "select rt_source, stop_sequence, schedule_relationship, arrival_delay, departure_delay \
         from rt_stop_time_update order by rt_source, trip_id, stop_sequence",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

fn seqs(stops: &[RtStop]) -> Vec<i32> {
    stops.iter().map(|s| s.stop_sequence).collect()
}

// ---- insert ----

#[sqlx::test]
async fn first_write_inserts_trips_and_stops(pool: PgPool) {
    let mut a = trip_update_row("A");
    a.trip_delay = Some(60);
    put(&pool, SRC, &[a], &[stop("A", 1), stop("A", 2)]).await;

    let trips = rt_trips(&pool).await;
    assert_eq!(trips.len(), 1);
    assert_eq!(trips[0].trip_id, "A");
    assert_eq!(trips[0].trip_delay, Some(60));
    assert!(trips[0].has_realtime);
    assert!(trips[0].rt_updated_at.is_some());
    assert_eq!(seqs(&rt_stops(&pool).await), [1, 2]);
}

#[sqlx::test]
async fn schedule_only_trip_has_null_rt_updated_at(pool: PgPool) {
    let mut a = trip_update_row("A");
    a.has_realtime = false;
    put(&pool, SRC, &[a], &[]).await;

    let trips = rt_trips(&pool).await;
    assert!(!trips[0].has_realtime);
    assert!(trips[0].rt_updated_at.is_none());
}

// ---- upsert: live feed wins ----

#[sqlx::test]
async fn live_poll_overwrites_delay_timestamp_and_vehicle(pool: PgPool) {
    let mut a = trip_update_row("A");
    (a.trip_delay, a.rt_timestamp, a.vehicle_id, a.vehicle_label) =
        (Some(60), Some(1000), Some("v1".into()), Some("L1".into()));
    put(&pool, SRC, &[a], &[]).await;

    let mut a2 = trip_update_row("A");
    (
        a2.trip_delay,
        a2.rt_timestamp,
        a2.vehicle_id,
        a2.vehicle_label,
    ) = (Some(120), Some(2000), Some("v2".into()), Some("L2".into()));
    put(&pool, SRC, &[a2], &[]).await;

    let trips = rt_trips(&pool).await;
    assert_eq!(trips.len(), 1);
    assert_eq!(trips[0].trip_delay, Some(120));
    assert_eq!(trips[0].rt_timestamp, Some(2000));
    assert_eq!(trips[0].vehicle_id.as_deref(), Some("v2"));
    assert_eq!(trips[0].vehicle_label.as_deref(), Some("L2"));
}

// ---- upsert: silent poll keeps last-known state ----

#[sqlx::test]
async fn silent_poll_keeps_last_known_realtime_values(pool: PgPool) {
    let mut a = trip_update_row("A");
    (a.trip_delay, a.rt_timestamp, a.vehicle_id) = (Some(60), Some(1000), Some("v1".into()));
    put(&pool, SRC, &[a], &[]).await;
    let before = rt_trips(&pool).await.remove(0);

    let mut silent = trip_update_row("A");
    silent.has_realtime = false;
    (silent.trip_delay, silent.rt_timestamp, silent.vehicle_id) = (None, None, None);
    put(&pool, SRC, &[silent], &[]).await;

    let after = rt_trips(&pool).await.remove(0);
    assert!(after.has_realtime, "has_realtime stays true once seen");
    assert_eq!(after.trip_delay, Some(60));
    assert_eq!(after.rt_timestamp, Some(1000));
    assert_eq!(after.vehicle_id.as_deref(), Some("v1"));
    assert_eq!(
        after.rt_updated_at, before.rt_updated_at,
        "rt_updated_at untouched by a silent poll"
    );
    assert!(
        after.updated_at >= before.updated_at,
        "updated_at still advances"
    );
}

// ---- schedule_relationship: a confident poll always wins ----
//
// No source's CANCELED/DELETED is trusted less than any other value it might
// report: a fresh, confident poll overwrites whatever came before, cancellation
// included. There's no legitimate source where the previous poll's claim
// should outrank the current one — a real GTFS-RT cancellation doesn't flip
// back to SCHEDULED on its own, so a poll that says it did is either a real
// operator correction (which should apply) or a source's own inferred
// CANCELED getting overridden by a better later match (which must be allowed,
// or a single bad poll would leave a genuinely-running trip reading canceled
// for the rest of the service day). Only a silent poll (no fresh
// `has_realtime` data) falls back to the last known value.

#[sqlx::test]
async fn a_canceled_relationship_is_overwritten_by_a_later_confident_poll(pool: PgPool) {
    let mut a = trip_update_row("A");
    a.schedule_relationship = 3;
    put(&pool, SRC, &[a], &[]).await;

    let mut back = trip_update_row("A");
    back.schedule_relationship = 0;
    put(&pool, SRC, &[back], &[]).await;

    assert_eq!(rt_trips(&pool).await[0].schedule_relationship, 0);
}

#[sqlx::test]
async fn a_deleted_relationship_is_overwritten_by_a_later_confident_poll(pool: PgPool) {
    let mut a = trip_update_row("A");
    a.schedule_relationship = 7;
    put(&pool, SRC, &[a], &[]).await;

    let mut back = trip_update_row("A");
    back.schedule_relationship = 0;
    put(&pool, SRC, &[back], &[]).await;

    assert_eq!(rt_trips(&pool).await[0].schedule_relationship, 0);
}

#[sqlx::test]
async fn a_silent_poll_still_keeps_the_last_known_relationship(pool: PgPool) {
    let mut a = trip_update_row("A");
    a.schedule_relationship = 3;
    put(&pool, SRC, &[a], &[]).await;

    let mut silent = trip_update_row("A");
    silent.has_realtime = false;
    silent.schedule_relationship = 0;
    put(&pool, SRC, &[silent], &[]).await;

    assert_eq!(
        rt_trips(&pool).await[0].schedule_relationship,
        3,
        "silence isn't a confident poll"
    );
}

#[sqlx::test]
async fn non_sticky_relationship_follows_live_feed(pool: PgPool) {
    let a = trip_update_row("A"); // schedule_relationship 0
    put(&pool, SRC, &[a], &[]).await;

    let mut a2 = trip_update_row("A");
    a2.schedule_relationship = 2; // unscheduled
    put(&pool, SRC, &[a2], &[]).await;

    assert_eq!(rt_trips(&pool).await[0].schedule_relationship, 2);
}

#[sqlx::test]
async fn silent_poll_does_not_change_relationship(pool: PgPool) {
    let mut a = trip_update_row("A");
    a.schedule_relationship = 2;
    put(&pool, SRC, &[a], &[]).await;

    let mut silent = trip_update_row("A");
    silent.has_realtime = false;
    silent.schedule_relationship = 0;
    put(&pool, SRC, &[silent], &[]).await;

    assert_eq!(rt_trips(&pool).await[0].schedule_relationship, 2);
}

// ---- trip missing from a poll is left alone ----

#[sqlx::test]
async fn trip_missing_from_new_snapshot_is_left_untouched(pool: PgPool) {
    let mut b = trip_update_row("B");
    b.trip_delay = Some(90);
    put(&pool, SRC, &[trip_update_row("A"), b], &[stop("B", 1)]).await;
    let b_before = rt_trips(&pool)
        .await
        .into_iter()
        .find(|t| t.trip_id == "B")
        .unwrap();

    // B has fallen out of this poll's candidates entirely (e.g. left the
    // running-now window) — it must not be deleted, nor even touched.
    put(&pool, SRC, &[trip_update_row("A")], &[]).await;

    let trips = rt_trips(&pool).await;
    assert_eq!(
        trips.iter().map(|t| t.trip_id.as_str()).collect::<Vec<_>>(),
        ["A", "B"],
        "B stays even though this poll didn't mention it"
    );
    let b_after = trips.into_iter().find(|t| t.trip_id == "B").unwrap();
    assert_eq!(b_after.trip_delay, Some(90));
    assert_eq!(
        b_after.updated_at, b_before.updated_at,
        "untouched, not just silent"
    );
    assert_eq!(seqs(&rt_stops(&pool).await), [1], "B's stop is kept too");
}

#[sqlx::test]
async fn writes_are_scoped_to_rt_source(pool: PgPool) {
    put(&pool, SRC, &[trip_update_row("A")], &[]).await;
    put(&pool, SRC2, &[trip_update_row("B")], &[]).await;
    put(&pool, SRC, &[trip_update_row("C")], &[]).await;

    let trips = rt_trips(&pool).await;
    assert_eq!(
        trips
            .iter()
            .map(|t| (t.rt_source.as_str(), t.trip_id.as_str()))
            .collect::<Vec<_>>(),
        [("vbb-alt", "B"), ("vbb-rt", "A"), ("vbb-rt", "C")],
        "A is kept (absence from a later SRC poll isn't a deletion) and SRC2 is untouched"
    );
}

// ---- stop upsert ----

#[sqlx::test]
async fn stop_values_are_replaced_by_the_fresh_snapshot(pool: PgPool) {
    let mut s1 = stop("A", 1);
    (s1.arrival_delay, s1.departure_delay) = (Some(60), Some(60));
    put(&pool, SRC, &[trip_update_row("A")], &[s1]).await;

    let mut s1b = stop("A", 1);
    (s1b.arrival_delay, s1b.departure_delay) = (Some(300), Some(330));
    put(&pool, SRC, &[trip_update_row("A")], &[s1b]).await;

    let stops = rt_stops(&pool).await;
    assert_eq!(stops.len(), 1);
    assert_eq!(
        (stops[0].arrival_delay, stops[0].departure_delay),
        (Some(300), Some(330))
    );
}

#[sqlx::test]
async fn skipped_stop_stays_skipped_while_unreported(pool: PgPool) {
    let mut s = stop("A", 2);
    s.schedule_relationship = 1;
    put(&pool, SRC, &[trip_update_row("A")], &[s]).await;

    // The feed stopped listing the (now past) stop; the row is only materialised
    let mut s2 = stop("A", 2);
    s2.reported = false;
    put(&pool, SRC, &[trip_update_row("A")], &[s2]).await;

    assert_eq!(rt_stops(&pool).await[0].schedule_relationship, 1);
}

#[sqlx::test]
async fn reported_stop_revises_an_earlier_skip(pool: PgPool) {
    let mut s = stop("A", 2);
    s.schedule_relationship = 1;
    put(&pool, SRC, &[trip_update_row("A")], &[s]).await;

    // The feed takes its short-turn prediction back and reports the stop again
    put(&pool, SRC, &[trip_update_row("A")], &[stop("A", 2)]).await;

    assert_eq!(rt_stops(&pool).await[0].schedule_relationship, 0);
}

#[sqlx::test]
async fn non_skipped_stop_relationship_follows_snapshot(pool: PgPool) {
    put(&pool, SRC, &[trip_update_row("A")], &[stop("A", 1)]).await;

    let mut s2 = stop("A", 1);
    s2.schedule_relationship = 2;
    put(&pool, SRC, &[trip_update_row("A")], &[s2]).await;

    assert_eq!(rt_stops(&pool).await[0].schedule_relationship, 2);
}

// ---- prune stops ----

#[sqlx::test]
async fn stop_dropped_by_an_actively_updated_trip_is_deleted(pool: PgPool) {
    put(
        &pool,
        SRC,
        &[trip_update_row("A")],
        &[stop("A", 1), stop("A", 2), stop("A", 3)],
    )
    .await;
    put(
        &pool,
        SRC,
        &[trip_update_row("A")],
        &[stop("A", 1), stop("A", 2)],
    )
    .await;

    assert_eq!(seqs(&rt_stops(&pool).await), [1, 2]);
}

#[sqlx::test]
async fn stops_of_a_now_silent_trip_are_kept(pool: PgPool) {
    put(
        &pool,
        SRC,
        &[trip_update_row("A")],
        &[stop("A", 1), stop("A", 2), stop("A", 3)],
    )
    .await;

    let mut silent = trip_update_row("A");
    silent.has_realtime = false;
    put(&pool, SRC, &[silent], &[]).await;

    assert_eq!(
        seqs(&rt_stops(&pool).await),
        [1, 2, 3],
        "sticky while the trip is silent"
    );
}

#[sqlx::test]
async fn stops_of_a_canceled_trip_are_deleted(pool: PgPool) {
    put(
        &pool,
        SRC,
        &[trip_update_row("A")],
        &[stop("A", 1), stop("A", 2)],
    )
    .await;

    let mut canceled = trip_update_row("A");
    canceled.schedule_relationship = 3;
    put(&pool, SRC, &[canceled], &[]).await;

    assert!(rt_stops(&pool).await.is_empty());
}

#[sqlx::test]
async fn prune_stops_is_scoped_to_rt_source(pool: PgPool) {
    put(&pool, SRC, &[trip_update_row("A")], &[stop("A", 1)]).await;
    put(&pool, SRC2, &[trip_update_row("A")], &[stop("A", 1)]).await;

    let mut canceled = trip_update_row("A");
    canceled.schedule_relationship = 3;
    put(&pool, SRC, &[canceled], &[]).await;

    let stops = rt_stops(&pool).await;
    assert_eq!(
        stops
            .iter()
            .map(|s| s.rt_source.as_str())
            .collect::<Vec<_>>(),
        ["vbb-alt"]
    );
}

// ---- prune stops: a source without full stop coverage never infers a drop ----
//
// A source that only reports the few stops around the vehicle's current
// position, no matter how long the trip runs, is *never* a complete snapshot
// of the trip's stops. Treating "missing from this poll" as "dropped from the trip"
// for a source like that would delete a stop's whole history the moment its
// live window moves past it (the actual production bug this guards against:
// a real trip's earlier stops, once genuinely reported, vanished from
// `rt_stop_time_update` on the very next poll).

#[sqlx::test]
async fn a_sparse_source_keeps_a_stop_missing_from_a_later_poll(pool: PgPool) {
    put_sparse(
        &pool,
        SRC,
        &[trip_update_row("A")],
        &[stop("A", 1), stop("A", 13)],
    )
    .await;

    // The next poll's live window has moved on; it only reports stop 15 now.
    put_sparse(&pool, SRC, &[trip_update_row("A")], &[stop("A", 15)]).await;

    assert_eq!(
        seqs(&rt_stops(&pool).await),
        [1, 13, 15],
        "1 and 13 must survive, not just 15"
    );
}

#[sqlx::test]
async fn a_sparse_source_still_drops_stops_of_a_canceled_trip(pool: PgPool) {
    put_sparse(
        &pool,
        SRC,
        &[trip_update_row("A")],
        &[stop("A", 1), stop("A", 13)],
    )
    .await;

    let mut canceled = trip_update_row("A");
    canceled.schedule_relationship = 3;
    put_sparse(&pool, SRC, &[canceled], &[]).await;

    assert!(
        rt_stops(&pool).await.is_empty(),
        "cancellation prunes regardless of coverage"
    );
}
