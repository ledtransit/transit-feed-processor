#![allow(dead_code)]

use chrono::NaiveDate;

pub use transit_feed_processor::realtime::apply::{StopUpdateRow, TripUpdateRow};
pub use transit_feed_processor::realtime::store::{Snapshot, serialize, write};

pub fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// A `TripUpdateRow` with sensible defaults (RT-backed, on-time, no vehicle).
pub fn trip_update_row(trip_id: &str) -> TripUpdateRow {
    TripUpdateRow {
        trip_id: trip_id.into(),
        service_date: ymd(2024, 6, 15),
        route_id: "R1".into(),
        start_time: 40_000,
        end_time: 41_000,
        schedule_relationship: 0,
        has_realtime: true,
        trip_delay: None,
        rt_timestamp: None,
        vehicle_id: None,
        vehicle_label: None,
    }
}

pub fn stop_update_row(trip_id: &str, stop_sequence: i32, stop_id: &str) -> StopUpdateRow {
    StopUpdateRow {
        trip_id: trip_id.into(),
        service_date: ymd(2024, 6, 15),
        stop_sequence,
        stop_id: stop_id.into(),
        schedule_relationship: 0,
        reported: true,
        arrival_delay: None,
        departure_delay: None,
        arrival_time: None,
        departure_time: None,
    }
}

/// Split a Postgres `COPY ... FROM STDIN` text body into rows of tab-separated
/// cells (`\N` left verbatim, the trailing newline dropped).
pub fn copy_rows(body: &str) -> Vec<Vec<&str>> {
    if body.is_empty() {
        return vec![];
    }
    body.strip_suffix('\n')
        .unwrap_or(body)
        .split('\n')
        .map(|line| line.split('\t').collect())
        .collect()
}

pub fn only_row(body: &str) -> Vec<&str> {
    let rows = copy_rows(body);
    assert_eq!(rows.len(), 1, "expected exactly one row in:\n{body}");
    rows.into_iter().next().unwrap()
}
