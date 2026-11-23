#![allow(dead_code, unused_imports)]

pub use chrono_tz::Tz;
pub use gtfs_structures::{ExtendedRouteType, RouteType};

pub use transit_feed_processor::feed::{
    GtfsFeedFilter, GtfsFeedFilterAction, GtfsFeedFilterCriteria, GtfsFeedFilterRoute,
};
pub use transit_feed_processor::schedule::filter::{Selected, select};
pub use transit_feed_processor::schedule::gtfs::{
    AgencyRow, CalendarDateRow, CalendarRow, RouteRow, StopRow, StopTimeRow, Tables, TripRow,
};
pub use transit_feed_processor::schedule::store::{CopyBundle, serialize};

pub const BERLIN: Tz = chrono_tz::Europe::Berlin;
pub const NEW_YORK: Tz = chrono_tz::America::New_York;

// ---- row builders ----

pub fn tables() -> Tables {
    Tables {
        agencies: vec![],
        routes: vec![],
        trips: vec![],
        stop_times: vec![],
        stops: vec![],
        calendar: vec![],
        calendar_dates: vec![],
    }
}

pub fn selected() -> Selected {
    Selected {
        agencies: vec![],
        routes: vec![],
        trips: vec![],
        stop_times: vec![],
        stops: vec![],
        calendar: vec![],
        calendar_dates: vec![],
    }
}

pub fn agency(id: &str, name: &str, timezone: &str) -> AgencyRow {
    AgencyRow {
        agency_id: id.into(),
        agency_name: name.into(),
        agency_timezone: timezone.into(),
    }
}

/// `route_type` as the `i16` GTFS code, mirroring gtfs-structures' own
/// `Serialize for RouteType` so the value round-trips through `parse_route_type`.
pub fn route_type_code(rt: RouteType) -> i16 {
    match rt {
        RouteType::Tramway => 0,
        RouteType::Subway => 1,
        RouteType::Rail => 2,
        RouteType::Bus => 3,
        RouteType::Ferry => 4,
        RouteType::CableCar => 5,
        RouteType::Gondola => 6,
        RouteType::Funicular => 7,
        RouteType::Extended(ext) => ext as i16,
        RouteType::Other(code) => code,
    }
}

pub fn route(
    id: &str,
    agency_id: Option<&str>,
    short_name: Option<&str>,
    long_name: Option<&str>,
    route_type: RouteType,
) -> RouteRow {
    RouteRow {
        route_id: id.into(),
        agency_id: agency_id.map(Into::into),
        route_short_name: short_name.map(Into::into),
        route_long_name: long_name.map(Into::into),
        route_desc: None,
        route_type: route_type_code(route_type),
        route_url: None,
        route_color: None,
        route_text_color: None,
        route_sort_order: None,
        continuous_pickup: None,
        continuous_drop_off: None,
        network_id: None,
    }
}

pub fn trip(trip_id: &str, route_id: &str, service_id: &str) -> TripRow {
    TripRow {
        route_id: route_id.into(),
        service_id: service_id.into(),
        trip_id: trip_id.into(),
        trip_headsign: None,
        trip_short_name: None,
        direction_id: None,
        block_id: None,
        shape_id: None,
        wheelchair_accessible: None,
        bikes_allowed: None,
    }
}

pub fn stop_time(trip_id: &str, stop_id: &str, stop_sequence: i32) -> StopTimeRow {
    StopTimeRow {
        trip_id: trip_id.into(),
        arrival_time: None,
        departure_time: None,
        stop_id: stop_id.into(),
        stop_sequence,
        stop_headsign: None,
        pickup_type: None,
        drop_off_type: None,
        continuous_pickup: None,
        continuous_drop_off: None,
        shape_dist_traveled: None,
        timepoint: None,
    }
}

pub fn stop_time_timed(
    trip_id: &str,
    stop_id: &str,
    stop_sequence: i32,
    arrival: Option<&str>,
    departure: Option<&str>,
) -> StopTimeRow {
    StopTimeRow {
        arrival_time: arrival.map(Into::into),
        departure_time: departure.map(Into::into),
        ..stop_time(trip_id, stop_id, stop_sequence)
    }
}

pub fn stop(stop_id: &str, parent_station: Option<&str>) -> StopRow {
    StopRow {
        stop_id: stop_id.into(),
        stop_code: None,
        stop_name: None,
        tts_stop_name: None,
        stop_desc: None,
        stop_lat: None,
        stop_lon: None,
        zone_id: None,
        stop_url: None,
        location_type: None,
        parent_station: parent_station.map(Into::into),
        stop_timezone: None,
        wheelchair_boarding: None,
        level_id: None,
        platform_code: None,
    }
}

pub fn calendar(service_id: &str) -> CalendarRow {
    CalendarRow {
        service_id: service_id.into(),
        monday: 1,
        tuesday: 1,
        wednesday: 1,
        thursday: 1,
        friday: 1,
        saturday: 0,
        sunday: 0,
        start_date: "20240101".into(),
        end_date: "20241231".into(),
    }
}

pub fn calendar_date(service_id: &str, date: &str, exception_type: i16) -> CalendarDateRow {
    CalendarDateRow {
        service_id: service_id.into(),
        date: date.into(),
        exception_type,
    }
}

// ---- filter builders (leak to satisfy the 'static criteria slices) ----

pub fn include(criteria: Vec<GtfsFeedFilterCriteria>) -> GtfsFeedFilter {
    GtfsFeedFilter {
        action: GtfsFeedFilterAction::Include,
        criteria: Vec::leak(criteria),
    }
}

pub fn exclude(criteria: Vec<GtfsFeedFilterCriteria>) -> GtfsFeedFilter {
    GtfsFeedFilter {
        action: GtfsFeedFilterAction::Exclude,
        criteria: Vec::leak(criteria),
    }
}

pub fn agencies(names: &[&'static str]) -> GtfsFeedFilterCriteria {
    GtfsFeedFilterCriteria::Agencies(Vec::leak(names.to_vec()))
}

pub fn route_of(typ: RouteType) -> GtfsFeedFilterCriteria {
    GtfsFeedFilterCriteria::RouteFilter(GtfsFeedFilterRoute { typ, prefix: None })
}

pub fn route_prefixed(typ: RouteType, prefix: &'static str) -> GtfsFeedFilterCriteria {
    GtfsFeedFilterCriteria::RouteFilter(GtfsFeedFilterRoute {
        typ,
        prefix: Some(prefix),
    })
}

pub fn any_of(subs: Vec<GtfsFeedFilterCriteria>) -> GtfsFeedFilterCriteria {
    GtfsFeedFilterCriteria::AnyOf(Vec::leak(subs))
}

// ---- assertion helpers ----

pub fn select_err(t: &Tables, filters: &[GtfsFeedFilter], tz: Tz) -> String {
    match select(t, filters, tz) {
        Ok(_) => panic!("select should have failed"),
        Err(e) => format!("{e:#}"),
    }
}

fn sorted<'a>(it: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut v: Vec<&str> = it.collect();
    v.sort_unstable();
    v
}

pub fn route_ids(sel: &Selected) -> Vec<&str> {
    sorted(sel.routes.iter().map(|r| r.route_id.as_str()))
}

pub fn trip_ids(sel: &Selected) -> Vec<&str> {
    sorted(sel.trips.iter().map(|t| t.trip_id.as_str()))
}

pub fn stop_ids(sel: &Selected) -> Vec<&str> {
    sorted(sel.stops.iter().map(|s| s.stop_id.as_str()))
}

pub fn agency_ids(sel: &Selected) -> Vec<&str> {
    sorted(sel.agencies.iter().map(|a| a.agency_id.as_str()))
}

pub fn calendar_service_ids(sel: &Selected) -> Vec<&str> {
    sorted(sel.calendar.iter().map(|c| c.service_id.as_str()))
}

pub fn calendar_date_service_ids(sel: &Selected) -> Vec<&str> {
    sorted(sel.calendar_dates.iter().map(|c| c.service_id.as_str()))
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

/// The single row of a body expected to hold exactly one.
pub fn only_row(body: &str) -> Vec<&str> {
    let rows = copy_rows(body);
    assert_eq!(rows.len(), 1, "expected exactly one row in:\n{body}");
    rows.into_iter().next().unwrap()
}
