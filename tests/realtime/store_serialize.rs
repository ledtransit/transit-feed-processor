//! Tests for `realtime::store::serialize` — rendering `TripUpdateRow` /
//! `StopUpdateRow` into the `COPY` bodies for the `_rt_trip` / `_rt_stop` staging
//! tables.

use crate::common::*;

const FEED: &str = "vbb";
const SCOPE: &str = "berlin_s_u";
const SRC: &str = "vbb-rt";

fn ser(trips: &[TripUpdateRow], stops: &[StopUpdateRow]) -> Snapshot {
    serialize(FEED, SCOPE, SRC, trips, stops, true)
}

// ---- trips ----

#[test]
fn trip_row_column_layout() {
    let mut t = trip_update_row("T1");
    t.route_id = "R7".into();
    t.start_time = 43_200;
    t.end_time = 46_800;
    t.schedule_relationship = 3;
    t.has_realtime = true;
    t.trip_delay = Some(120);
    t.rt_timestamp = Some(1_899_999_999);
    t.vehicle_id = Some("veh-1".into());
    t.vehicle_label = Some("Bus 1".into());

    let snap = ser(&[t], &[]);
    assert_eq!(
        only_row(&snap.trips),
        [
            "vbb",
            "berlin_s_u",
            "vbb-rt",
            "T1",
            "2024-06-15",
            "R7",
            "43200",
            "46800",
            "3",
            "t",
            "120",
            "1899999999",
            "veh-1",
            "Bus 1",
        ],
    );
    assert!(snap.trips.ends_with('\n'));
    assert!(snap.stops.is_empty());
}

#[test]
fn trip_row_optional_fields_render_as_null() {
    let mut t = trip_update_row("T1");
    t.has_realtime = false;

    let snap = ser(&[t], &[]);
    let row = only_row(&snap.trips);
    assert_eq!(row[9], "f"); // has_realtime
    assert_eq!(row[10], "\\N"); // trip_delay
    assert_eq!(row[11], "\\N"); // rt_timestamp
    assert_eq!(row[12], "\\N"); // vehicle_id
    assert_eq!(row[13], "\\N"); // vehicle_label
}

#[test]
fn negative_trip_delay_keeps_its_sign() {
    let mut t = trip_update_row("T1");
    t.trip_delay = Some(-90);
    let snap = ser(&[t], &[]);
    assert_eq!(only_row(&snap.trips)[10], "-90");
}

// ---- stops ----

#[test]
fn stop_row_column_layout() {
    let mut s = stop_update_row("T1", 4, "stopX");
    s.schedule_relationship = 1;
    s.arrival_delay = Some(60);
    s.departure_delay = Some(75);
    s.arrival_time = Some(1_700_000_060);
    s.departure_time = Some(1_700_000_075);

    let snap = ser(&[], &[s]);
    assert_eq!(
        only_row(&snap.stops),
        [
            "vbb",
            "berlin_s_u",
            "vbb-rt",
            "T1",
            "2024-06-15",
            "4",
            "stopX",
            "1",
            "t",
            "60",
            "75",
            "1700000060",
            "1700000075",
        ],
    );
}

#[test]
fn stop_row_optional_fields_render_as_null() {
    let snap = ser(&[], &[stop_update_row("T1", 1, "s1")]);
    let row = only_row(&snap.stops);
    assert_eq!(row[7], "0"); // schedule_relationship
    assert_eq!(row[8], "t"); // reported
    assert_eq!(&row[9..13], ["\\N", "\\N", "\\N", "\\N"]);
}

#[test]
fn negative_stop_delays_keep_their_sign() {
    let mut s = stop_update_row("T1", 1, "s1");
    s.arrival_delay = Some(-120);
    s.departure_delay = Some(-30);
    let snap = ser(&[], &[s]);
    let row = only_row(&snap.stops);
    assert_eq!((row[9], row[10]), ("-120", "-30"));
}

// ---- shape ----

#[test]
fn every_row_is_prefixed_with_feed_scope_and_source() {
    let trips = vec![trip_update_row("A"), trip_update_row("B")];
    let stops = vec![stop_update_row("A", 1, "s1"), stop_update_row("A", 2, "s2")];
    let snap = ser(&trips, &stops);
    for body in [&snap.trips, &snap.stops] {
        for row in copy_rows(body) {
            assert_eq!(&row[0..3], ["vbb", "berlin_s_u", "vbb-rt"]);
        }
    }
}

#[test]
fn body_has_one_newline_terminated_line_per_row() {
    let trips = vec![
        trip_update_row("A"),
        trip_update_row("B"),
        trip_update_row("C"),
    ];
    let body = ser(&trips, &[]).trips;
    assert_eq!(body.matches('\n').count(), 3);
    assert!(body.ends_with('\n'));
    assert_eq!(copy_rows(&body).len(), 3);
}

#[test]
fn empty_inputs_yield_empty_bodies() {
    let snap = ser(&[], &[]);
    assert!(snap.trips.is_empty());
    assert!(snap.stops.is_empty());
    assert_eq!(snap.feed, "vbb");
    assert_eq!(snap.scope, "berlin_s_u");
    assert_eq!(snap.rt_source, "vbb-rt");
}

#[test]
fn text_fields_escape_control_chars() {
    let mut t = trip_update_row("A\tB");
    t.vehicle_label = Some("Line\\1".into());
    let snap = ser(&[t], &[]);
    let row = only_row(&snap.trips);
    assert_eq!(row.len(), 14);
    assert_eq!(row[3], "A\\tB");
    assert_eq!(row[13], "Line\\\\1");
}
