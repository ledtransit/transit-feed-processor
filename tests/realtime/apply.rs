//! Tests for `realtime::apply::apply` — overlaying gtfs-rt `TripUpdate`s onto the
//! static candidate trips. gtfs-rt reuses the static `trip_id` / `stop_id`, so
//! matching is exact and every case here is built from plain ids.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use transit_feed_processor::realtime::apply::{StopUpdateRow, TripUpdateRow, apply};
use transit_feed_processor::realtime::candidates::{CandidateStop, CandidateTrip};
use transit_feed_processor::realtime::proto::gtfs_realtime::trip_update::{
    StopTimeEvent, StopTimeUpdate,
};
use transit_feed_processor::realtime::proto::gtfs_realtime::{
    TripDescriptor, TripUpdate, VehicleDescriptor,
};

const TZ: chrono_tz::Tz = chrono_tz::Europe::Berlin;

// TripDescriptor.ScheduleRelationship
const TRIP_CANCELED: i32 = 3;
const TRIP_DELETED: i32 = 7;
// StopTimeUpdate.ScheduleRelationship
const STOP_SKIPPED: i32 = 1;
const STOP_NO_DATA: i32 = 2;

const MAX_DELAY_SECS: i32 = 6 * 3600;

/// A local wall time on [`day`] as a UTC instant.
fn at(hour: u32, min: u32) -> DateTime<Utc> {
    TZ.with_ymd_and_hms(2024, 6, 15, hour, min, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

/// Reference "now": 21:12 local — between the stops the delay fixtures schedule
/// at 21:00 / 21:10 / 21:20 / 21:30. Candidate stops without a scheduled time
/// are unaffected by it.
fn now() -> DateTime<Utc> {
    at(21, 12)
}

fn run(
    candidates: &[CandidateTrip],
    updates: &[TripUpdate],
) -> (Vec<TripUpdateRow>, Vec<StopUpdateRow>) {
    apply(TZ, candidates, updates, now())
}

fn run_at(
    candidates: &[CandidateTrip],
    updates: &[TripUpdate],
    when: DateTime<Utc>,
) -> (Vec<TripUpdateRow>, Vec<StopUpdateRow>) {
    apply(TZ, candidates, updates, when)
}

// ---- builders ----

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 6, 15).unwrap()
}

fn cand_on(trip_id: &str, service_date: NaiveDate, stops: Vec<CandidateStop>) -> CandidateTrip {
    CandidateTrip {
        trip_id: trip_id.into(),
        route_id: "R1".into(),
        route_short_name: None,
        agency_name: None,
        direction_id: None,
        trip_headsign: None,
        service_date,
        start_time: 40_000,
        end_time: 41_000,
        stops,
    }
}

fn cand(trip_id: &str, stops: Vec<CandidateStop>) -> CandidateTrip {
    cand_on(trip_id, day(), stops)
}

fn cstop(seq: i32, stop_id: &str) -> CandidateStop {
    CandidateStop {
        stop_sequence: seq,
        stop_id: stop_id.into(),
        stop_name: None,
        stop_lat: None,
        stop_lon: None,
        arrival_time: None,
        departure_time: None,
    }
}

fn cstop_sched(seq: i32, stop_id: &str, secs: i32) -> CandidateStop {
    CandidateStop {
        arrival_time: Some(secs),
        departure_time: Some(secs),
        ..cstop(seq, stop_id)
    }
}

/// Stops `s1..sN`, sequences `1..=n`, no scheduled times.
fn seq_stops(n: i32) -> Vec<CandidateStop> {
    (1..=n).map(|s| cstop(s, &format!("s{s}"))).collect()
}

/// Four stops scheduled 21:00 / 21:10 / 21:20 / 21:30 local — [`now`] (21:12)
/// sits between seq 2 and seq 3.
fn timed_stops() -> Vec<CandidateStop> {
    vec![
        cstop_sched(1, "s1", 21 * 3600),
        cstop_sched(2, "s2", 21 * 3600 + 600),
        cstop_sched(3, "s3", 21 * 3600 + 1200),
        cstop_sched(4, "s4", 21 * 3600 + 1800),
    ]
}

fn stop_delay(stops: &[StopUpdateRow], seq: i32) -> Option<i32> {
    stops
        .iter()
        .find(|s| s.stop_sequence == seq)
        .unwrap()
        .arrival_delay
}

fn tu(trip_id: &str) -> TripUpdate {
    TripUpdate {
        trip: TripDescriptor {
            trip_id: Some(trip_id.into()),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn tu_delay(trip_id: &str, delay: i32) -> TripUpdate {
    TripUpdate {
        delay: Some(delay),
        ..tu(trip_id)
    }
}

fn stu(seq: u32) -> StopTimeUpdate {
    StopTimeUpdate {
        stop_sequence: Some(seq),
        ..Default::default()
    }
}

fn ev(delay: i32) -> StopTimeEvent {
    StopTimeEvent {
        delay: Some(delay),
        ..Default::default()
    }
}

fn ev_at(time: i64) -> StopTimeEvent {
    StopTimeEvent {
        time: Some(time),
        ..Default::default()
    }
}

fn trip_row<'a>(rows: &'a [TripUpdateRow], id: &str) -> &'a TripUpdateRow {
    rows.iter()
        .find(|r| r.trip_id == id)
        .expect("missing trip row")
}

// ---- schedule-only vs realtime ----

#[test]
fn candidate_without_matching_update_is_schedule_only() {
    let cands = [cand("A", seq_stops(2))];
    let (trips, stops) = run(&cands, &[tu_delay("B", 60)]);

    assert_eq!(trips.len(), 1);
    let a = &trips[0];
    assert_eq!(a.trip_id, "A");
    assert!(!a.has_realtime);
    assert_eq!(a.trip_delay, None);
    assert_eq!(a.schedule_relationship, 0);
    assert_eq!(a.rt_timestamp, None);
    assert_eq!(a.vehicle_id, None);
    assert!(stops.is_empty());
}

#[test]
fn one_trip_row_per_candidate_in_candidate_order() {
    let cands = [cand("A", vec![]), cand("C", vec![]), cand("B", vec![])];
    let (trips, _) = run(&cands, &[tu("A"), tu("B")]);

    assert_eq!(
        trips.iter().map(|t| t.trip_id.as_str()).collect::<Vec<_>>(),
        ["A", "C", "B"]
    );
    assert!(trip_row(&trips, "A").has_realtime);
    assert!(!trip_row(&trips, "C").has_realtime);
    assert!(trip_row(&trips, "B").has_realtime);
}

#[test]
fn matching_update_without_stop_updates_materializes_stop_rows_from_trip_delay() {
    let cands = [cand("A", seq_stops(3))];
    let (trips, stops) = run(&cands, &[tu_delay("A", 120)]);

    assert!(trip_row(&trips, "A").has_realtime);
    assert_eq!(trip_row(&trips, "A").trip_delay, Some(120));
    assert_eq!(stops.len(), 3);
    for s in &stops {
        assert_eq!(s.arrival_delay, Some(120));
        assert_eq!(s.departure_delay, Some(120));
        assert_eq!(s.arrival_time, None);
        assert_eq!(s.departure_time, None);
        assert_eq!(s.schedule_relationship, 0);
    }
}

#[test]
fn running_update_on_stopless_candidate_makes_no_stop_rows() {
    let cands = [cand("A", vec![])];
    let (trips, stops) = run(&cands, &[tu_delay("A", 77)]);

    assert!(trip_row(&trips, "A").has_realtime);
    assert_eq!(trip_row(&trips, "A").trip_delay, Some(77));
    assert!(stops.is_empty());
}

// ---- pick_update: start_date disambiguation ----

#[test]
fn update_with_matching_start_date_is_preferred() {
    let cands = [cand_on("A", day(), vec![])];
    let mut wrong = tu_delay("A", 60);
    wrong.trip.start_date = Some("20240614".into());
    let dateless = tu_delay("A", 10);
    let mut right = tu_delay("A", 999);
    right.trip.start_date = Some("20240615".into());

    let (trips, _) = run(&cands, &[wrong, dateless, right]);
    assert_eq!(trip_row(&trips, "A").trip_delay, Some(999));
}

#[test]
fn dateless_update_applies_when_no_start_date_matches() {
    let cands = [cand_on("A", day(), vec![])];
    let mut wrong = tu_delay("A", 60);
    wrong.trip.start_date = Some("20240614".into());
    let dateless = tu_delay("A", 10);

    let (trips, _) = run(&cands, &[wrong, dateless]);
    assert_eq!(trip_row(&trips, "A").trip_delay, Some(10));
}

#[test]
fn update_naming_another_date_is_ignored() {
    let cands = [cand_on("A", day(), seq_stops(2))];
    let mut wrong = tu_delay("A", 60);
    wrong.trip.start_date = Some("20240614".into());

    let (trips, stops) = run(&cands, &[wrong]);
    assert!(!trip_row(&trips, "A").has_realtime);
    assert_eq!(trip_row(&trips, "A").trip_delay, None);
    assert!(stops.is_empty());
}

// ---- canceled / deleted ----

#[test]
fn canceled_trip_yields_trip_row_only() {
    let cands = [cand("A", seq_stops(2))];
    let mut u = tu_delay("A", 300);
    u.trip.schedule_relationship = Some(TRIP_CANCELED);
    u.stop_time_update = vec![StopTimeUpdate {
        arrival: Some(ev(999)),
        ..stu(1)
    }];

    let (trips, stops) = run(&cands, &[u]);
    let a = trip_row(&trips, "A");
    assert_eq!(a.schedule_relationship, TRIP_CANCELED as i16);
    assert!(a.has_realtime);
    assert_eq!(a.trip_delay, Some(300));
    assert!(stops.is_empty());
}

#[test]
fn deleted_trip_yields_trip_row_only() {
    let cands = [cand("A", seq_stops(2))];
    let mut u = tu("A");
    u.trip.schedule_relationship = Some(TRIP_DELETED);

    let (trips, stops) = run(&cands, &[u]);
    assert_eq!(
        trip_row(&trips, "A").schedule_relationship,
        TRIP_DELETED as i16
    );
    assert!(stops.is_empty());
}

// ---- delay forward-fill ----

#[test]
fn only_stops_with_a_stop_time_update_are_reported() {
    let cands = [cand("A", seq_stops(4))];
    let mut u = tu_delay("A", 60);
    u.stop_time_update = vec![stu(2), stu(3)];

    let (_, stops) = run(&cands, &[u]);
    let reported: Vec<bool> = stops.iter().map(|s| s.reported).collect();
    assert_eq!(reported, [false, true, true, false]);
}

#[test]
fn stop_delay_overrides_trip_delay_from_that_stop_onward() {
    let cands = [cand("A", seq_stops(4))];
    let mut u = tu_delay("A", 60);
    u.stop_time_update = vec![StopTimeUpdate {
        arrival: Some(ev(300)),
        departure: Some(ev(300)),
        ..stu(3)
    }];

    let (_, stops) = run(&cands, &[u]);
    assert_eq!(delays(&stops), [(60, 60), (60, 60), (300, 300), (300, 300)]);
}

#[test]
fn no_data_stop_clears_carried_delay() {
    let cands = [cand("A", seq_stops(4))];
    let mut u = tu_delay("A", 90);
    u.stop_time_update = vec![StopTimeUpdate {
        schedule_relationship: Some(STOP_NO_DATA),
        ..stu(3)
    }];

    let (trips, stops) = run(&cands, &[u]);
    assert_eq!(stops[0].arrival_delay, Some(90));
    assert_eq!(
        (stops[2].arrival_delay, stops[2].departure_delay),
        (None, None)
    );
    assert_eq!(stops[2].schedule_relationship, STOP_NO_DATA as i16);
    assert_eq!(
        (stops[3].arrival_delay, stops[3].departure_delay),
        (None, None)
    );
    // trip delay still falls back to the explicit trip-level value
    assert_eq!(trip_row(&trips, "A").trip_delay, Some(90));
}

#[test]
fn skipped_stop_keeps_carried_delay_propagating() {
    let cands = [cand("A", seq_stops(4))];
    let mut u = tu_delay("A", 45);
    u.stop_time_update = vec![StopTimeUpdate {
        schedule_relationship: Some(STOP_SKIPPED),
        ..stu(2)
    }];

    let (_, stops) = run(&cands, &[u]);
    assert_eq!(delays(&stops), [(45, 45), (45, 45), (45, 45), (45, 45)]);
    assert_eq!(stops[1].schedule_relationship, STOP_SKIPPED as i16);
}

#[test]
fn skipped_stop_ignores_its_own_delay_event() {
    let cands = [cand("A", seq_stops(3))];
    let mut u = tu_delay("A", 30);
    u.stop_time_update = vec![StopTimeUpdate {
        schedule_relationship: Some(STOP_SKIPPED),
        arrival: Some(ev(999)),
        departure: Some(ev(999)),
        ..stu(2)
    }];

    let (_, stops) = run(&cands, &[u]);
    assert_eq!(delays(&stops), [(30, 30), (30, 30), (30, 30)]);
}

// ---- stop matching: sequence then stop_id ----

#[test]
fn stop_update_matched_by_stop_id_without_sequence() {
    let cands = [cand("A", vec![cstop(1, "p"), cstop(2, "q"), cstop(3, "r")])];
    let mut u = tu("A");
    u.stop_time_update = vec![StopTimeUpdate {
        stop_id: Some("q".into()),
        arrival: Some(ev(200)),
        departure: Some(ev(200)),
        ..Default::default()
    }];

    let (_, stops) = run(&cands, &[u]);
    assert_eq!(stops[0].arrival_delay, None);
    assert_eq!(stops[1].arrival_delay, Some(200));
    assert_eq!(stops[2].arrival_delay, Some(200)); // carried
}

#[test]
fn stop_sequence_match_beats_stop_id_match() {
    let cands = [cand("A", vec![cstop(1, "x"), cstop(2, "x")])];
    let mut u = tu("A");
    u.stop_time_update = vec![
        StopTimeUpdate {
            arrival: Some(ev(500)),
            departure: Some(ev(500)),
            ..stu(2)
        },
        StopTimeUpdate {
            stop_id: Some("x".into()),
            arrival: Some(ev(111)),
            departure: Some(ev(111)),
            ..Default::default()
        },
    ];

    let (_, stops) = run(&cands, &[u]);
    assert_eq!(stops[0].arrival_delay, Some(111)); // seq 1 → by stop_id
    assert_eq!(stops[1].arrival_delay, Some(500)); // seq 2 → by stop_sequence
}

// ---- effective_delay: explicit delay vs time-vs-schedule ----

#[test]
fn event_time_without_delay_is_measured_against_schedule() {
    let sched = Utc
        .with_ymd_and_hms(2024, 6, 15, 10, 0, 0)
        .unwrap()
        .timestamp(); // 12:00 CEST
    let cands = [cand("A", vec![cstop_sched(1, "s1", 43_200)])];
    let mut u = tu("A");
    u.stop_time_update = vec![StopTimeUpdate {
        arrival: Some(ev_at(sched + 180)),
        ..stu(1)
    }];

    let (_, stops) = run(&cands, &[u]);
    assert_eq!(stops[0].arrival_delay, Some(180));
    assert_eq!(stops[0].arrival_time, Some(sched + 180));
}

#[test]
fn explicit_event_delay_wins_but_raw_time_is_kept() {
    let cands = [cand("A", vec![cstop_sched(1, "s1", 43_200)])];
    let mut u = tu("A");
    u.stop_time_update = vec![StopTimeUpdate {
        arrival: Some(StopTimeEvent {
            delay: Some(120),
            time: Some(999_999),
            ..Default::default()
        }),
        ..stu(1)
    }];

    let (_, stops) = run(&cands, &[u]);
    assert_eq!(stops[0].arrival_delay, Some(120));
    assert_eq!(stops[0].arrival_time, Some(999_999));
}

#[test]
fn implausible_delay_is_ignored() {
    let cands = [cand("A", seq_stops(1))];
    let mut u = tu("A");
    u.stop_time_update = vec![StopTimeUpdate {
        arrival: Some(ev(MAX_DELAY_SECS + 1)),
        ..stu(1)
    }];

    let (_, stops) = run(&cands, &[u]);
    assert_eq!(stops[0].arrival_delay, None);
    assert_eq!(stops[0].arrival_time, None);
}

#[test]
fn delay_at_the_plausibility_boundary_is_kept() {
    let cands = [cand("A", seq_stops(1))];
    let mut u = tu("A");
    u.stop_time_update = vec![StopTimeUpdate {
        arrival: Some(ev(MAX_DELAY_SECS)),
        ..stu(1)
    }];

    let (_, stops) = run(&cands, &[u]);
    assert_eq!(stops[0].arrival_delay, Some(MAX_DELAY_SECS));
}

#[test]
fn event_time_without_a_scheduled_time_yields_no_delay() {
    let cands = [cand("A", seq_stops(1))]; // cstop has no arrival_time
    let mut u = tu("A");
    u.stop_time_update = vec![StopTimeUpdate {
        arrival: Some(ev_at(1_700_000_000)),
        ..stu(1)
    }];

    let (_, stops) = run(&cands, &[u]);
    assert_eq!(stops[0].arrival_delay, None);
    assert_eq!(stops[0].arrival_time, None);
}

// ---- trip_delay fallback ----

#[test]
fn trip_delay_falls_back_to_last_carried_departure() {
    let cands = [cand("A", seq_stops(2))];
    let mut u = tu("A"); // no trip-level delay
    u.stop_time_update = vec![StopTimeUpdate {
        departure: Some(ev(240)),
        ..stu(2)
    }];

    let (trips, _) = run(&cands, &[u]);
    assert_eq!(trip_row(&trips, "A").trip_delay, Some(240));
}

#[test]
fn trip_delay_falls_back_to_arrival_carry_when_no_departure_seen() {
    let cands = [cand("A", seq_stops(2))];
    let mut u = tu("A");
    u.stop_time_update = vec![StopTimeUpdate {
        arrival: Some(ev(300)),
        ..stu(1)
    }];

    let (trips, _) = run(&cands, &[u]);
    assert_eq!(trip_row(&trips, "A").trip_delay, Some(300));
}

#[test]
fn trip_delay_is_the_next_stops_delay_not_a_recovered_tail() {
    // No TripUpdate.delay. Stop 2 is +5 min; the feed forecasts the terminus
    // (stop 4) back to on-time. `now` sits between stop 2 and stop 3, so stop 3 —
    // the next stop the vehicle reaches — still carries the +5 min.
    let cands = [cand("A", timed_stops())];
    let mut u = tu("A");
    u.stop_time_update = vec![
        StopTimeUpdate {
            arrival: Some(ev(300)),
            ..stu(2)
        },
        StopTimeUpdate {
            arrival: Some(ev(0)),
            ..stu(4)
        },
    ];

    let (trips, stops) = run(&cands, &[u]);
    assert_eq!(
        trip_row(&trips, "A").trip_delay,
        Some(300),
        "the next stop, not the on-time forecast"
    );
    // The forecast is untouched in the stop rows.
    assert_eq!(stop_delay(&stops, 3), Some(300));
    assert_eq!(stop_delay(&stops, 4), Some(0));
}

#[test]
fn trip_delay_falls_back_to_the_worst_stop_once_the_trip_is_over() {
    let cands = [cand("A", timed_stops())];
    let mut u = tu("A");
    u.stop_time_update = vec![
        StopTimeUpdate {
            arrival: Some(ev(300)),
            ..stu(2)
        },
        StopTimeUpdate {
            arrival: Some(ev(120)),
            ..stu(4)
        },
    ];

    // Every scheduled stop is in the past. The last stop's own 120s reads
    // on-time-ish next to seq2's 300s — exactly the tail-recovery forecast
    // the fallback must not trust — so it picks the worse of the two instead.
    let (trips, _) = run_at(&cands, &[u], at(23, 0));
    assert_eq!(trip_row(&trips, "A").trip_delay, Some(300));
}

#[test]
fn trip_delay_uses_the_first_stop_before_the_trip_has_started() {
    let cands = [cand("A", timed_stops())];
    let mut u = tu("A");
    u.stop_time_update = vec![StopTimeUpdate {
        arrival: Some(ev(180)),
        ..stu(1)
    }];

    // now is before every scheduled stop.
    let (trips, _) = run_at(&cands, &[u], at(20, 0));
    assert_eq!(trip_row(&trips, "A").trip_delay, Some(180));
}

#[test]
fn explicit_trip_level_delay_still_wins_over_the_next_stop() {
    let cands = [cand("A", timed_stops())];
    let mut u = tu_delay("A", 600);
    u.stop_time_update = vec![StopTimeUpdate {
        arrival: Some(ev(60)),
        ..stu(3)
    }];

    let (trips, _) = run(&cands, &[u]);
    assert_eq!(trip_row(&trips, "A").trip_delay, Some(600));
}

// ---- vehicle / timestamp passthrough ----

#[test]
fn vehicle_and_timestamp_are_copied() {
    let cands = [cand("A", vec![])];
    let mut u = tu("A");
    u.timestamp = Some(1_700_000_000);
    u.vehicle = Some(VehicleDescriptor {
        id: Some("veh-7".into()),
        label: Some("Bus 7".into()),
        ..Default::default()
    });

    let (trips, _) = run(&cands, &[u]);
    let a = trip_row(&trips, "A");
    assert_eq!(a.rt_timestamp, Some(1_700_000_000));
    assert_eq!(a.vehicle_id.as_deref(), Some("veh-7"));
    assert_eq!(a.vehicle_label.as_deref(), Some("Bus 7"));
}

#[test]
fn empty_vehicle_id_and_label_become_none() {
    let cands = [cand("A", vec![])];
    let mut u = tu("A");
    u.vehicle = Some(VehicleDescriptor {
        id: Some(String::new()),
        label: Some(String::new()),
        ..Default::default()
    });

    let (trips, _) = run(&cands, &[u]);
    let a = trip_row(&trips, "A");
    assert_eq!(a.vehicle_id, None);
    assert_eq!(a.vehicle_label, None);
}

fn delays(stops: &[StopUpdateRow]) -> Vec<(i32, i32)> {
    stops
        .iter()
        .map(|s| (s.arrival_delay.unwrap(), s.departure_delay.unwrap()))
        .collect()
}
