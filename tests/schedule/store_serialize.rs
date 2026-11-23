use crate::common::*;

const FEED: &str = "vbb";
const SCOPE: &str = "berlin_s_u";

fn ser(data: Selected) -> CopyBundle {
    serialize(FEED, SCOPE, data).expect("serialize should succeed")
}

// ---- column layout ----

#[test]
fn agency_row_is_feed_and_scope_prefixed() {
    let mut d = selected();
    d.agencies = vec![agency("1", "Berliner Verkehrsbetriebe", "Europe/Berlin")];
    let b = ser(d);
    assert_eq!(b.feed, FEED);
    assert_eq!(b.scope, SCOPE);
    assert_eq!(
        only_row(&b.agencies),
        [
            "vbb",
            "berlin_s_u",
            "1",
            "Berliner Verkehrsbetriebe",
            "Europe/Berlin"
        ],
    );
    assert!(b.agencies.ends_with('\n'));
}

#[test]
fn every_table_row_starts_with_feed_and_scope() {
    let mut d = selected();
    d.agencies = vec![agency("1", "A", "Europe/Berlin")];
    d.routes = vec![route("r", Some("1"), Some("R"), None, RouteType::Bus)];
    d.trips = vec![trip("t", "r", "svc")];
    d.stop_times = vec![stop_time_timed(
        "t",
        "s",
        1,
        Some("06:00:00"),
        Some("06:00:00"),
    )];
    d.stops = vec![stop("s", None)];
    d.calendar = vec![calendar("svc")];
    d.calendar_dates = vec![calendar_date("svc", "20240501", 1)];
    let b = ser(d);
    for body in [
        &b.agencies,
        &b.routes,
        &b.trips,
        &b.stop_times,
        &b.stops,
        &b.calendar,
        &b.calendar_dates,
    ] {
        for row in copy_rows(body) {
            assert_eq!(&row[0..2], ["vbb", "berlin_s_u"], "in body:\n{body}");
        }
    }
}

#[test]
fn body_has_one_newline_terminated_line_per_row() {
    let mut d = selected();
    d.agencies = vec![
        agency("1", "A", "Europe/Berlin"),
        agency("2", "B", "Europe/Berlin"),
        agency("3", "C", "Europe/Berlin"),
    ];
    let body = ser(d).agencies;
    assert_eq!(body.matches('\n').count(), 3);
    assert!(body.ends_with('\n'));
    assert_eq!(copy_rows(&body).len(), 3);
}

#[test]
fn empty_selected_produces_empty_bodies() {
    let b = ser(selected());
    for body in [
        &b.agencies,
        &b.routes,
        &b.stops,
        &b.calendar,
        &b.calendar_dates,
        &b.trips,
        &b.stop_times,
    ] {
        assert!(body.is_empty());
    }
}

// ---- routes ----

#[test]
fn route_color_defaults_when_blank_or_missing() {
    let mut d = selected();
    let missing = route("m", None, Some("M"), None, RouteType::Bus);
    let mut blank = route("b", None, Some("B"), None, RouteType::Bus);
    blank.route_color = Some(String::new());
    blank.route_text_color = Some(String::new());
    let mut set = route("s", None, Some("S"), None, RouteType::Bus);
    set.route_color = Some("12AB34".into());
    set.route_text_color = Some("FEDCBA".into());
    d.routes = vec![missing, blank, set];

    let b = ser(d);
    let rows = copy_rows(&b.routes);
    assert_eq!((rows[0][9], rows[0][10]), ("FFFFFF", "000000"));
    assert_eq!((rows[1][9], rows[1][10]), ("FFFFFF", "000000"));
    assert_eq!((rows[2][9], rows[2][10]), ("12AB34", "FEDCBA"));
}

#[test]
fn route_optional_fields_render_as_null() {
    let mut d = selected();
    d.routes = vec![route("r", None, Some("R"), None, RouteType::Bus)];
    let b = ser(d);
    let row = only_row(&b.routes);
    assert_eq!(row[3], "\\N"); // agency_id
    assert_eq!(row[6], "\\N"); // route_desc
    assert_eq!(row[7], "3"); // route_type (Bus)
    assert_eq!(row[8], "\\N"); // route_url
    assert_eq!(row[11], "\\N"); // route_sort_order
    assert_eq!(row[14], "\\N"); // network_id
}

// ---- stop_times ----

#[test]
fn clock_times_become_seconds_after_midnight() {
    let mut d = selected();
    d.stop_times = vec![
        stop_time_timed("t1", "s1", 1, Some("01:00:30"), Some("01:01:00")),
        stop_time_timed("t1", "s2", 2, None, Some("25:30:00")),
    ];
    let b = ser(d);
    let rows = copy_rows(&b.stop_times);
    assert_eq!((rows[0][5], rows[0][6]), ("3630", "3660"));
    assert_eq!((rows[1][5], rows[1][6]), ("\\N", "91800"));
}

#[test]
fn unparseable_clock_time_renders_null() {
    let mut d = selected();
    d.stop_times = vec![stop_time_timed("t1", "s1", 1, Some("nope"), Some("07:30"))];
    let b = ser(d);
    let row = only_row(&b.stop_times);
    assert_eq!((row[5], row[6]), ("\\N", "\\N"));
}

// ---- trip spans (join of stop_times into trips) ----

#[test]
fn trip_span_is_derived_from_its_stop_times() {
    let mut d = selected();
    d.trips = vec![trip("t1", "r1", "svc")];
    d.stop_times = vec![
        stop_time_timed("t1", "s1", 1, None, Some("06:00:00")),
        stop_time_timed("t1", "s2", 2, Some("06:30:00"), Some("06:31:00")),
        stop_time_timed("t1", "s3", 3, Some("07:00:00"), None),
    ];
    let b = ser(d);
    let row = only_row(&b.trips);
    assert_eq!((row[12], row[13]), ("21600", "25200"));
}

#[test]
fn trip_without_stop_times_has_null_span() {
    let mut d = selected();
    d.trips = vec![trip("lonely", "r1", "svc")];
    let b = ser(d);
    let row = only_row(&b.trips);
    assert_eq!((row[12], row[13]), ("\\N", "\\N"));
}

// ---- calendar / calendar_dates ----

#[test]
fn calendar_weekday_bitmap_and_iso_dates() {
    let mut d = selected();
    let weekdays = calendar("wk");
    let mut odd = calendar("odd");
    odd.monday = 2;
    odd.tuesday = 0;
    d.calendar = vec![weekdays, odd];

    let b = ser(d);
    let rows = copy_rows(&b.calendar);
    assert_eq!(&rows[0][3..10], ["t", "t", "t", "t", "t", "f", "f"]);
    assert_eq!((rows[0][10], rows[0][11]), ("2024-01-01", "2024-12-31"));
    assert_eq!(rows[1][3], "f"); // monday = 2 is not exactly 1
}

#[test]
fn calendar_invalid_date_is_an_error() {
    let mut d = selected();
    let mut bad = calendar("wk");
    bad.start_date = "2024-01-01".into();
    d.calendar = vec![bad];

    let err = match serialize(FEED, SCOPE, d) {
        Ok(_) => panic!("expected serialize to fail"),
        Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("invalid date"), "{err}");
}

#[test]
fn calendar_date_row_layout() {
    let mut d = selected();
    d.calendar_dates = vec![calendar_date("hol", "20240501", 2)];
    let b = ser(d);
    assert_eq!(
        only_row(&b.calendar_dates),
        ["vbb", "berlin_s_u", "hol", "2024-05-01", "2"],
    );
}

// ---- escaping ----

#[test]
fn text_cells_escape_control_chars() {
    let mut d = selected();
    d.agencies = vec![agency("1", "A\tB\\C", "Europe/Berlin")];
    let mut s = stop("s1", None);
    s.stop_name = Some("Line1\nLine2".into());
    d.stops = vec![s];
    let b = ser(d);

    let arow = only_row(&b.agencies);
    assert_eq!(arow.len(), 5);
    assert_eq!(arow[3], "A\\tB\\\\C");

    let srow = only_row(&b.stops);
    assert_eq!(srow.len(), 17);
    assert_eq!(srow[4], "Line1\\nLine2");
}
