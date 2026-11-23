use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;

use super::candidates::{CandidateStop, CandidateTrip};
use super::proto::gtfs_realtime::TripUpdate;
use super::proto::gtfs_realtime::trip_descriptor::ScheduleRelationship as TripRel;
use super::proto::gtfs_realtime::trip_update::stop_time_update::ScheduleRelationship as StopRel;
use super::proto::gtfs_realtime::trip_update::{StopTimeEvent, StopTimeUpdate};

/// A derived delay larger than this is treated as bad data
const MAX_DELAY_SECS: i32 = 6 * 3600;

pub struct TripUpdateRow {
    pub trip_id: String,
    pub service_date: NaiveDate,
    pub route_id: String,
    pub start_time: i32,
    pub end_time: i32,
    /// Raw `TripDescriptor.ScheduleRelationship` value (0 when absent).
    pub schedule_relationship: i16,
    pub has_realtime: bool,
    pub trip_delay: Option<i32>,
    pub rt_timestamp: Option<i64>,
    pub vehicle_id: Option<String>,
    pub vehicle_label: Option<String>,
}

pub struct StopUpdateRow {
    pub trip_id: String,
    pub service_date: NaiveDate,
    pub stop_sequence: i32,
    pub stop_id: String,
    /// Raw `StopTimeUpdate.ScheduleRelationship` value (0 when absent).
    pub schedule_relationship: i16,
    /// Whether the source reported on this stop this poll, rather than the row
    /// being materialised from the trip's schedule and carried delays. Only a
    /// reported stop may change its stored `schedule_relationship`.
    pub reported: bool,
    pub arrival_delay: Option<i32>,
    pub departure_delay: Option<i32>,
    pub arrival_time: Option<i64>,
    pub departure_time: Option<i64>,
}

/// Overlay realtime `trip_updates` onto the static `candidates` (using each
/// candidate's own nested `stops`), producing rows for the `rt_trip_update` /
/// `rt_stop_time_update` tables.
///
/// - every candidate yields one `TripUpdateRow` (schedule-only candidates get
///   `has_realtime = false` and no stop rows);
/// - `CANCELED` / `DELETED` trips get the trip row only;
/// - stop rows are materialised per static stop with delays forward-filled along
///   the trip (`SKIPPED` keeps the carried delay, `NO_DATA` clears it).
///
/// `now` is the moment the feed snapshot describes; when the feed sets no
/// `TripUpdate.delay` the trip-level delay is taken from the next stop the
/// vehicle will reach rather than the last stop, whose delay is often forecast
/// back to on-time.
pub fn apply(
    tz: Tz,
    candidates: &[CandidateTrip],
    trip_updates: &[TripUpdate],
    now: DateTime<Utc>,
) -> (Vec<TripUpdateRow>, Vec<StopUpdateRow>) {
    let now = now.timestamp();
    let mut updates: HashMap<&str, Vec<&TripUpdate>> = HashMap::new();
    for tu in trip_updates {
        if let Some(id) = tu.trip.trip_id.as_deref() {
            updates.entry(id).or_default().push(tu);
        }
    }

    let mut trip_rows = Vec::with_capacity(candidates.len());
    let mut stop_rows = Vec::new();

    for c in candidates {
        let update = pick_update(updates.get(c.trip_id.as_str()), c.service_date);
        let raw_rel = update
            .and_then(|u| u.trip.schedule_relationship)
            .unwrap_or(0);
        let mut trip_delay = update.and_then(|u| u.delay);

        if let Some(update) = update {
            let runs = !matches!(
                TripRel::try_from(raw_rel),
                Ok(TripRel::Canceled | TripRel::Deleted)
            );
            if runs && !c.stops.is_empty() {
                let mut by_seq: HashMap<i32, &StopTimeUpdate> = HashMap::new();
                let mut by_stop: HashMap<&str, &StopTimeUpdate> = HashMap::new();
                for stu in &update.stop_time_update {
                    if let Some(seq) = stu.stop_sequence {
                        by_seq.insert(seq as i32, stu);
                    }
                    if let Some(id) = stu.stop_id.as_deref() {
                        by_stop.insert(id, stu);
                    }
                }

                // trip-level delay seeds the carry; a StopTimeUpdate delay overrides it
                // from that stop onward (GTFS-RT semantics).
                let mut carried_arr = update.delay;
                let mut carried_dep = update.delay;
                let mut trip_stop_rows = Vec::with_capacity(c.stops.len());

                for s in &c.stops {
                    let stu = by_seq
                        .get(&s.stop_sequence)
                        .or_else(|| by_stop.get(s.stop_id.as_str()))
                        .copied();
                    let raw_stop_rel = stu.and_then(|u| u.schedule_relationship).unwrap_or(0);

                    let mut arrival_time = None;
                    let mut departure_time = None;

                    match StopRel::try_from(raw_stop_rel) {
                        Ok(StopRel::NoData) => {
                            carried_arr = None;
                            carried_dep = None;
                        }
                        Ok(StopRel::Skipped) => { /* carried delays keep propagating */ }
                        _ => {
                            if let Some(stu) = stu {
                                if let Some(ev) = &stu.arrival
                                    && let Some(d) =
                                        effective_delay(tz, c.service_date, s.arrival_time, ev)
                                {
                                    carried_arr = Some(d);
                                    arrival_time = ev.time;
                                }
                                if let Some(ev) = &stu.departure
                                    && let Some(d) =
                                        effective_delay(tz, c.service_date, s.departure_time, ev)
                                {
                                    carried_dep = Some(d);
                                    departure_time = ev.time;
                                }
                            }
                        }
                    }

                    trip_stop_rows.push(StopUpdateRow {
                        trip_id: c.trip_id.clone(),
                        service_date: c.service_date,
                        stop_sequence: s.stop_sequence,
                        stop_id: s.stop_id.clone(),
                        schedule_relationship: raw_stop_rel as i16,
                        reported: stu.is_some(),
                        arrival_delay: carried_arr,
                        departure_delay: carried_dep,
                        arrival_time,
                        departure_time,
                    });
                }

                // TripUpdate.delay is optional and some sources never set it; fall
                // back to the delay in force at the next stop the vehicle will
                // reach (not the last stop — feeds forecast the tail to on-time).
                trip_delay = trip_delay
                    .or_else(|| delay_at_now(tz, c.service_date, &c.stops, &trip_stop_rows, now));
                stop_rows.extend(trip_stop_rows);
            }
        }

        trip_rows.push(TripUpdateRow {
            trip_id: c.trip_id.clone(),
            service_date: c.service_date,
            route_id: c.route_id.clone(),
            start_time: c.start_time,
            end_time: c.end_time,
            schedule_relationship: raw_rel as i16,
            has_realtime: update.is_some(),
            trip_delay,
            rt_timestamp: update.and_then(|u| u.timestamp).map(|t| t as i64),
            vehicle_id: update
                .and_then(|u| u.vehicle.as_ref())
                .and_then(|v| nonempty(v.id.as_deref())),
            vehicle_label: update
                .and_then(|u| u.vehicle.as_ref())
                .and_then(|v| nonempty(v.label.as_deref())),
        });
    }

    (trip_rows, stop_rows)
}

/// Pick the `TripUpdate` for a candidate: one whose `start_date` matches the
/// service date, else one without a `start_date` (applies to any instance).
/// An update that names a *different* date is for another instance of the same
/// `trip_id` and is not applied.
fn pick_update<'a>(
    updates: Option<&'a Vec<&'a TripUpdate>>,
    service_date: NaiveDate,
) -> Option<&'a TripUpdate> {
    let list = updates?;
    let wanted = service_date.format("%Y%m%d").to_string();
    list.iter()
        .find(|u| u.trip.start_date.as_deref() == Some(wanted.as_str()))
        .or_else(|| list.iter().find(|u| u.trip.start_date.is_none()))
        .copied()
}

/// Effective delay for one predicted event: the feed's explicit `delay`, else
/// `predicted time - scheduled time`. `None` if neither is available or the
/// result is implausibly large (bad data).
fn effective_delay(
    tz: Tz,
    date: NaiveDate,
    scheduled_secs: Option<i32>,
    event: &StopTimeEvent,
) -> Option<i32> {
    let delay = match event.delay {
        Some(d) => d,
        None => (event.time? - scheduled_unix(tz, date, scheduled_secs?)?) as i32,
    };
    (delay.abs() <= MAX_DELAY_SECS).then_some(delay)
}

/// The forward-filled delay at the first stop the vehicle has not reached yet
/// (scheduled at or after `now`), else the worst delay seen anywhere in the
/// trip. Used as the trip-level delay only when the feed set no
/// `TripUpdate.delay`: the tail of a trip is routinely forecast back to
/// on-time, so simply falling back to the last stop understates how late the
/// vehicle is right now — and that fallback is needed most exactly when the
/// vehicle is running so late that its *entire* raw (undelayed) schedule
/// already reads as past, which is precisely when trusting an optimistic
/// tail forecast is most wrong. The worst-seen delay is a safer floor: an
/// early stop's real, already-observed lateness is better evidence of how
/// late the vehicle still is than a later stop's hopeful recovery forecast.
fn delay_at_now(
    tz: Tz,
    date: NaiveDate,
    stops: &[CandidateStop],
    rows: &[StopUpdateRow],
    now: i64,
) -> Option<i32> {
    let ahead = stops
        .iter()
        .zip(rows)
        .find(|(s, _)| {
            s.arrival_time
                .or(s.departure_time)
                .and_then(|secs| scheduled_unix(tz, date, secs))
                .is_some_and(|at| at >= now)
        })
        .and_then(|(_, row)| row.arrival_delay.or(row.departure_delay));

    ahead.or_else(|| {
        rows.iter()
            .filter_map(|row| row.arrival_delay.or(row.departure_delay))
            .max()
    })
}

/// A scheduled seconds-after-service-date-midnight value (may exceed 86400) as
/// a POSIX timestamp in `tz`. Shared with the plugin's fuzzy realtime matchers,
/// which compare their own predicted times against the static schedule the same
/// way gtfs-rt delays are derived above.
pub(crate) fn scheduled_unix(tz: Tz, date: NaiveDate, secs: i32) -> Option<i64> {
    let naive = date.and_hms_opt(0, 0, 0)? + Duration::seconds(secs as i64);
    Some(tz.from_local_datetime(&naive).single()?.timestamp())
}

fn nonempty(s: Option<&str>) -> Option<String> {
    s.filter(|s| !s.is_empty()).map(str::to_owned)
}
