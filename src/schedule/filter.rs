use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use chrono_tz::Tz;
use gtfs_structures::{ExtendedRouteType, RouteType};

use super::gtfs::{
    AgencyRow, CalendarDateRow, CalendarRow, RouteRow, RowSource, StopRow, StopTimeRow, Tables,
    TripRow,
};
use crate::feed::{GtfsFeedFilter, GtfsFeedFilterAction, GtfsFeedFilterCriteria};

#[derive(Default)]
pub struct Selected {
    pub agencies: Vec<AgencyRow>,
    pub routes: Vec<RouteRow>,
    pub trips: Vec<TripRow>,
    pub stop_times: Vec<StopTimeRow>,
    pub stops: Vec<StopRow>,
    pub calendar: Vec<CalendarRow>,
    pub calendar_dates: Vec<CalendarDateRow>,
}

/// Filters are OR-combined; a filter matches when all its criteria match.
/// Route selection drives everything else: trips of kept routes, stop_times of
/// kept trips, stops referenced by those stop_times (plus parent stations), and
/// services referenced by kept trips.
///
/// After route selection, every agency still referenced must declare `timezone`
/// (IANA form, e.g. `Europe/Berlin`) — the realtime window math assumes a single
/// feed timezone.
pub fn select(t: &Tables, filters: &[GtfsFeedFilter], timezone: Tz) -> Result<Selected> {
    let mut selected = [Selected::default()];
    select_scopes(&mut &*t, &[filters], timezone, &mut selected)?;
    let [selected] = selected;
    Ok(selected)
}

/// Where [`select_scopes`] hands a scope's selected rows, table by table in
/// the order given here, each table's rows in feed order.
pub trait ScopeSink {
    fn agencies(&mut self, rows: Vec<AgencyRow>);
    fn routes(&mut self, rows: Vec<RouteRow>);
    fn trip(&mut self, row: TripRow);
    fn stop_time(&mut self, row: StopTimeRow);
    fn stop(&mut self, row: StopRow);
    fn calendar(&mut self, row: CalendarRow);
    fn calendar_date(&mut self, row: CalendarDateRow);
}

/// Collects the rows.
impl ScopeSink for Selected {
    fn agencies(&mut self, rows: Vec<AgencyRow>) {
        self.agencies = rows;
    }
    fn routes(&mut self, rows: Vec<RouteRow>) {
        self.routes = rows;
    }
    fn trip(&mut self, row: TripRow) {
        self.trips.push(row);
    }
    fn stop_time(&mut self, row: StopTimeRow) {
        self.stop_times.push(row);
    }
    fn stop(&mut self, row: StopRow) {
        self.stops.push(row);
    }
    fn calendar(&mut self, row: CalendarRow) {
        self.calendar.push(row);
    }
    fn calendar_date(&mut self, row: CalendarDateRow) {
        self.calendar_dates.push(row);
    }
}

/// Which of up to [`MAX_SCOPES`] scopes a row belongs to, one bit each.
type Scopes = u64;
const MAX_SCOPES: usize = Scopes::BITS as usize;

/// [`select`] for several scopes at once (each given by its filters), in one
/// pass over each table of `src` rather than one per scope: every row is
/// looked at once and handed to the sink of each scope it belongs to (`out`,
/// one per scope). Only rows some scope keeps are ever parsed, and a sink
/// needn't hold them, so a streaming source needs next to no memory for the
/// feed itself.
pub fn select_scopes(
    src: &mut impl RowSource,
    scopes: &[&[GtfsFeedFilter]],
    timezone: Tz,
    out: &mut [impl ScopeSink],
) -> Result<()> {
    if scopes.len() > MAX_SCOPES {
        bail!("at most {MAX_SCOPES} scopes per feed");
    }
    assert_eq!(scopes.len(), out.len(), "one sink per scope");
    let all_agencies = src.agencies()?;
    let all_routes = src.routes()?;

    // Routes and agencies per scope, and each kept route's scopes
    let mut route_scopes: HashMap<String, Scopes> = HashMap::new();
    for (i, (filters, sink)) in scopes.iter().zip(out.iter_mut()).enumerate() {
        let (agencies, routes) = select_routes(&all_agencies, &all_routes, filters, timezone)?;
        for r in &routes {
            *route_scopes.entry(r.route_id.clone()).or_default() |= 1 << i;
        }
        sink.agencies(agencies);
        sink.routes(routes);
    }
    drop((all_agencies, all_routes));

    // Keep trips of the kept routes, and note their trips' and services' scopes
    let mut trip_scopes: HashMap<String, Scopes> = HashMap::new();
    let mut service_scopes: HashMap<String, Scopes> = HashMap::new();
    src.trips(
        &|route_id| route_scopes.contains_key(route_id),
        &mut |trip| {
            let scopes = route_scopes[&trip.route_id];
            *trip_scopes.entry(trip.trip_id.clone()).or_default() |= scopes;
            *service_scopes.entry(trip.service_id.clone()).or_default() |= scopes;
            distribute(&mut *out, scopes, trip, ScopeSink::trip);
        },
    )?;

    // Keep stop_times of the kept trips, and note the stops they reference
    let mut stop_scopes: HashMap<String, Scopes> = HashMap::new();
    src.stop_times(&|trip_id| trip_scopes.contains_key(trip_id), &mut |st| {
        let scopes = trip_scopes[&st.trip_id];
        match stop_scopes.get_mut(&st.stop_id) {
            Some(known) => *known |= scopes,
            None => {
                stop_scopes.insert(st.stop_id.clone(), scopes);
            }
        }
        distribute(&mut *out, scopes, st, ScopeSink::stop_time);
    })?;
    drop(trip_scopes);

    // Keep the stops referenced, plus any parent stations: each pass over the
    // stops adds the parents of those found so far
    loop {
        let mut parents: Vec<(String, Scopes)> = Vec::new();
        src.stops(&|stop_id| stop_scopes.contains_key(stop_id), &mut |s| {
            let Some(parent) = s.parent_station.filter(|p| !p.is_empty()) else {
                return;
            };
            let scopes = stop_scopes[&s.stop_id];
            let known = stop_scopes.get(&parent).copied().unwrap_or(0);
            if scopes & !known != 0 {
                parents.push((parent, scopes));
            }
        })?;
        if parents.is_empty() {
            break;
        }
        for (parent, scopes) in parents {
            *stop_scopes.entry(parent).or_default() |= scopes;
        }
    }
    src.stops(&|stop_id| stop_scopes.contains_key(stop_id), &mut |s| {
        let scopes = stop_scopes[&s.stop_id];
        distribute(&mut *out, scopes, s, ScopeSink::stop);
    })?;

    // Keep calendar and calendar_dates of the kept trips' services
    src.calendar(&|id| service_scopes.contains_key(id), &mut |c| {
        let scopes = service_scopes[&c.service_id];
        distribute(&mut *out, scopes, c, ScopeSink::calendar);
    })?;
    src.calendar_dates(&|id| service_scopes.contains_key(id), &mut |c| {
        let scopes = service_scopes[&c.service_id];
        distribute(&mut *out, scopes, c, ScopeSink::calendar_date);
    })?;

    Ok(())
}

/// Hand `row` to the sink of every scope in `scopes` with `push`, copying it
/// only for all but the last.
fn distribute<S, T: Clone>(out: &mut [S], scopes: Scopes, row: T, push: impl Fn(&mut S, T)) {
    let last = (Scopes::BITS - 1 - scopes.leading_zeros()) as usize;
    for (i, sink) in out.iter_mut().enumerate().take(last) {
        if scopes & (1 << i) != 0 {
            push(sink, row.clone());
        }
    }
    push(&mut out[last], row);
}

/// The routes `filters` keep, in feed order, and their agencies, checked to
/// be in `timezone`.
fn select_routes(
    all_agencies: &[AgencyRow],
    all_routes: &[RouteRow],
    filters: &[GtfsFeedFilter],
    timezone: Tz,
) -> Result<(Vec<AgencyRow>, Vec<RouteRow>)> {
    let agency_by_id: HashMap<&str, &str> = all_agencies
        .iter()
        .map(|a| (a.agency_id.as_str(), a.agency_name.as_str()))
        .collect();
    let sole_agency = match all_agencies {
        [only] => Some(only.agency_name.as_str()),
        _ => None,
    };
    let ctx = Ctx {
        agency_by_id,
        sole_agency,
    };

    let has_include = filters
        .iter()
        .any(|f| matches!(f.action, GtfsFeedFilterAction::Include));

    // Determine whether a route should be kept based on the filters
    // A route is kept if it matches at least one include filter (or there are no include filters) and does not match any exclude filter.
    let keep_route = |r: &RouteRow| {
        let included = !has_include
            || filters.iter().any(|f| {
                matches!(f.action, GtfsFeedFilterAction::Include) && all_match(r, f.criteria, &ctx)
            });
        let excluded = filters.iter().any(|f| {
            matches!(f.action, GtfsFeedFilterAction::Exclude) && all_match(r, f.criteria, &ctx)
        });
        included && !excluded
    };

    // Filter routes using criteria
    let routes: Vec<RouteRow> = all_routes
        .iter()
        .filter(|r| keep_route(r))
        .cloned()
        .collect();

    // Check every agency referenced matches the feed's timezone
    let referenced_agencies = referenced_agency_ids(&routes, all_agencies);
    check_agency_timezones(all_agencies, &referenced_agencies, timezone)?;

    // Keep agencies of the kept routes
    let agencies: Vec<AgencyRow> = all_agencies
        .iter()
        .filter(|a| referenced_agencies.contains(a.agency_id.as_str()))
        .cloned()
        .collect();

    Ok((agencies, routes))
}

/// Ids of every agency referenced by a kept route, resolving an empty
/// `agency_id` to the feed's sole agency (only valid when there is exactly one).
fn referenced_agency_ids<'a>(routes: &[RouteRow], agencies: &'a [AgencyRow]) -> HashSet<&'a str> {
    let mut referenced: HashSet<&str> = HashSet::new();
    for r in routes {
        match r.agency_id.as_deref().filter(|id| !id.is_empty()) {
            Some(id) => {
                if let Some(a) = agencies.iter().find(|a| a.agency_id == id) {
                    referenced.insert(a.agency_id.as_str());
                }
            }
            None => {
                if let [only] = agencies {
                    referenced.insert(only.agency_id.as_str());
                }
            }
        }
    }
    referenced
}

/// Every agency referenced by a kept route must declare the feed's timezone.
fn check_agency_timezones(
    agencies: &[AgencyRow],
    referenced: &HashSet<&str>,
    expected: Tz,
) -> Result<()> {
    let want = expected.name();

    let mismatched: Vec<String> = agencies
        .iter()
        .filter(|a| referenced.contains(a.agency_id.as_str()))
        .filter(|a| a.agency_timezone != want)
        .map(|a| format!("{} ({})", a.agency_name, a.agency_timezone))
        .collect();

    if mismatched.is_empty() {
        Ok(())
    } else {
        bail!(
            "agency timezone mismatch: feed timezone is {want}, but {}",
            mismatched.join(", ")
        );
    }
}

struct Ctx<'a> {
    agency_by_id: HashMap<&'a str, &'a str>,
    sole_agency: Option<&'a str>,
}

fn all_match(r: &RouteRow, criteria: &[GtfsFeedFilterCriteria], ctx: &Ctx) -> bool {
    criteria.iter().all(|c| criterion_match(r, c, ctx))
}

fn criterion_match(r: &RouteRow, c: &GtfsFeedFilterCriteria, ctx: &Ctx) -> bool {
    match c {
        GtfsFeedFilterCriteria::Agencies(names) => {
            route_agency(r, ctx).is_some_and(|name| names.contains(&name))
        }
        GtfsFeedFilterCriteria::RouteFilter(rf) => {
            standard(parse_route_type(r.route_type)) == standard(rf.typ)
                && rf
                    .prefix
                    .is_none_or(|p| route_name(r).is_some_and(|n| n.starts_with(p)))
        }
        GtfsFeedFilterCriteria::AnyOf(subs) => subs.iter().any(|s| criterion_match(r, s, ctx)),
    }
}

fn route_agency<'a>(r: &'a RouteRow, ctx: &Ctx<'a>) -> Option<&'a str> {
    match r.agency_id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => ctx.agency_by_id.get(id).copied(),
        None => ctx.sole_agency,
    }
}

fn route_name(r: &RouteRow) -> Option<&str> {
    r.route_short_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(r.route_long_name.as_deref())
}

fn parse_route_type(code: i16) -> RouteType {
    if let Ok(ext) = ExtendedRouteType::try_from(code) {
        return RouteType::Extended(ext);
    }
    match code {
        0 => RouteType::Tramway,
        1 => RouteType::Subway,
        2 => RouteType::Rail,
        3 => RouteType::Bus,
        4 => RouteType::Ferry,
        5 => RouteType::CableCar,
        6 => RouteType::Gondola,
        7 => RouteType::Funicular,
        other => RouteType::Other(other),
    }
}

fn standard(rt: RouteType) -> RouteType {
    match rt {
        RouteType::Extended(ext) => ext.to_standard_route_type(),
        other => other,
    }
}
