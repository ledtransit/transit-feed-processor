use crate::common::*;

/// One Berlin agency, three routes (subway / rail / bus) with a blank agency_id.
fn basic_world() -> Tables {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![
        route("sub", None, Some("U1"), None, RouteType::Subway),
        route("rail", None, Some("S1"), None, RouteType::Rail),
        route("bus", None, Some("B1"), None, RouteType::Bus),
    ];
    t
}

fn run(t: &Tables, filters: Vec<GtfsFeedFilter>) -> Selected {
    select(t, &filters, BERLIN).expect("select should succeed")
}

// ---- filter combination ----

#[test]
fn no_filters_keeps_every_route() {
    let sel = run(&basic_world(), vec![]);
    assert_eq!(route_ids(&sel), ["bus", "rail", "sub"]);
}

#[test]
fn include_filter_keeps_only_matches() {
    let sel = run(
        &basic_world(),
        vec![include(vec![route_of(RouteType::Subway)])],
    );
    assert_eq!(route_ids(&sel), ["sub"]);
}

#[test]
fn include_filters_are_or_combined() {
    let sel = run(
        &basic_world(),
        vec![
            include(vec![route_of(RouteType::Subway)]),
            include(vec![route_of(RouteType::Rail)]),
        ],
    );
    assert_eq!(route_ids(&sel), ["rail", "sub"]);
}

#[test]
fn criteria_within_a_filter_are_and_combined() {
    let mut t = tables();
    t.agencies = vec![
        agency("a", "Agency A", "Europe/Berlin"),
        agency("b", "Agency B", "Europe/Berlin"),
    ];
    t.routes = vec![
        route("keep", Some("a"), Some("U1"), None, RouteType::Subway),
        route(
            "wrong_agency",
            Some("b"),
            Some("U2"),
            None,
            RouteType::Subway,
        ),
        route("wrong_type", Some("a"), Some("B1"), None, RouteType::Bus),
    ];
    let sel = run(
        &t,
        vec![include(vec![
            agencies(&["Agency A"]),
            route_of(RouteType::Subway),
        ])],
    );
    assert_eq!(route_ids(&sel), ["keep"]);
}

#[test]
fn lone_exclude_filter_drops_matches_keeps_rest() {
    let sel = run(
        &basic_world(),
        vec![exclude(vec![route_of(RouteType::Bus)])],
    );
    assert_eq!(route_ids(&sel), ["rail", "sub"]);
}

#[test]
fn exclude_overrides_include() {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![
        route("U1", None, Some("U1"), None, RouteType::Subway),
        route("U8", None, Some("U8"), None, RouteType::Subway),
    ];
    let sel = run(
        &t,
        vec![
            include(vec![route_of(RouteType::Subway)]),
            exclude(vec![route_prefixed(RouteType::Subway, "U8")]),
        ],
    );
    assert_eq!(route_ids(&sel), ["U1"]);
}

#[test]
fn include_matching_nothing_yields_empty_selection() {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![route("bus", Some("a"), Some("B1"), None, RouteType::Bus)];
    t.trips = vec![trip("t1", "bus", "svc")];
    t.stop_times = vec![stop_time("t1", "s1", 1)];
    t.stops = vec![stop("s1", None)];
    t.calendar = vec![calendar("svc")];

    let sel = run(&t, vec![include(vec![agencies(&["Nonexistent"])])]);
    assert!(route_ids(&sel).is_empty());
    assert!(trip_ids(&sel).is_empty());
    assert!(stop_ids(&sel).is_empty());
    assert!(calendar_service_ids(&sel).is_empty());
}

// ---- Agencies criterion ----

#[test]
fn agency_matched_by_name_via_id() {
    let mut t = tables();
    t.agencies = vec![
        agency("bvg", "Berliner Verkehrsbetriebe", "Europe/Berlin"),
        agency("sbahn", "S-Bahn Berlin GmbH", "Europe/Berlin"),
    ];
    t.routes = vec![
        route("r1", Some("bvg"), Some("r1"), None, RouteType::Bus),
        route("r2", Some("sbahn"), Some("r2"), None, RouteType::Bus),
    ];
    let sel = run(&t, vec![include(vec![agencies(&["S-Bahn Berlin GmbH"])])]);
    assert_eq!(route_ids(&sel), ["r2"]);
    assert_eq!(agency_ids(&sel), ["sbahn"]);
}

#[test]
fn blank_agency_id_resolves_to_sole_agency() {
    let mut t = tables();
    t.agencies = vec![agency("solo", "Solo Agency", "Europe/Berlin")];
    t.routes = vec![route("x", None, Some("X1"), None, RouteType::Bus)];
    let sel = run(&t, vec![include(vec![agencies(&["Solo Agency"])])]);
    assert_eq!(route_ids(&sel), ["x"]);
}

#[test]
fn blank_agency_id_with_multiple_agencies_matches_nothing() {
    let mut t = tables();
    t.agencies = vec![
        agency("a", "Agency A", "Europe/Berlin"),
        agency("b", "Agency B", "Europe/Berlin"),
    ];
    t.routes = vec![route("x", Some(""), Some("X1"), None, RouteType::Bus)];
    let sel = run(&t, vec![include(vec![agencies(&["Agency A"])])]);
    assert!(route_ids(&sel).is_empty());
}

#[test]
fn unknown_agency_id_matches_nothing() {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![route("x", Some("ghost"), Some("X1"), None, RouteType::Bus)];
    let sel = run(&t, vec![include(vec![agencies(&["Agency A"])])]);
    assert!(route_ids(&sel).is_empty());
}

// ---- RouteFilter criterion ----

#[test]
fn route_type_matches_standard_code() {
    let sel = run(
        &basic_world(),
        vec![include(vec![route_of(RouteType::Rail)])],
    );
    assert_eq!(route_ids(&sel), ["rail"]);
}

#[test]
fn extended_route_type_is_standardized() {
    // SuburbanRailway collapses to Rail, Underground to Subway.
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![
        route(
            "S1",
            None,
            Some("S1"),
            None,
            RouteType::Extended(ExtendedRouteType::SuburbanRailway),
        ),
        route(
            "U2",
            None,
            Some("U2"),
            None,
            RouteType::Extended(ExtendedRouteType::Underground),
        ),
        route("B5", None, Some("B5"), None, RouteType::Bus),
    ];
    let sel = run(
        &t,
        vec![include(vec![any_of(vec![
            route_prefixed(RouteType::Rail, "S"),
            route_prefixed(RouteType::Subway, "U"),
        ])])],
    );
    assert_eq!(route_ids(&sel), ["S1", "U2"]);
}

#[test]
fn unknown_route_type_does_not_match_standard_filter() {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![
        route("weird", None, Some("W1"), None, RouteType::Other(42)),
        route("rail", None, Some("S1"), None, RouteType::Rail),
    ];
    let sel = run(&t, vec![include(vec![route_of(RouteType::Rail)])]);
    assert_eq!(route_ids(&sel), ["rail"]);
}

#[test]
fn prefix_matches_short_name() {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![
        route("u1", None, Some("U1"), None, RouteType::Subway),
        route("u8", None, Some("U8"), None, RouteType::Subway),
        route("m10", None, Some("M10"), None, RouteType::Subway),
    ];
    let sel = run(
        &t,
        vec![include(vec![route_prefixed(RouteType::Subway, "U")])],
    );
    assert_eq!(route_ids(&sel), ["u1", "u8"]);
}

#[test]
fn prefix_falls_back_to_long_name_when_short_blank() {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![
        route("r1", None, Some(""), Some("Sonderzug"), RouteType::Rail),
        route("r2", None, None, Some("Regionalbahn"), RouteType::Rail),
    ];
    let sel = run(
        &t,
        vec![include(vec![route_prefixed(RouteType::Rail, "S")])],
    );
    assert_eq!(route_ids(&sel), ["r1"]);
}

#[test]
fn prefix_mismatch_drops_route() {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![route("s1", None, Some("S1"), None, RouteType::Subway)];
    let sel = run(
        &t,
        vec![include(vec![route_prefixed(RouteType::Subway, "U")])],
    );
    assert!(route_ids(&sel).is_empty());
}

// ---- AnyOf ----

#[test]
fn any_of_matches_when_one_sub_matches() {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![
        route("sub", None, Some("U1"), None, RouteType::Subway),
        route("rail", None, Some("S1"), None, RouteType::Rail),
        route("bus", None, Some("B1"), None, RouteType::Bus),
    ];
    let sel = run(
        &t,
        vec![include(vec![any_of(vec![
            route_of(RouteType::Subway),
            route_of(RouteType::Rail),
        ])])],
    );
    assert_eq!(route_ids(&sel), ["rail", "sub"]);
}

#[test]
fn nested_any_of() {
    let sel = run(
        &basic_world(),
        vec![include(vec![any_of(vec![
            any_of(vec![route_of(RouteType::Subway)]),
            route_of(RouteType::Rail),
        ])])],
    );
    assert_eq!(route_ids(&sel), ["rail", "sub"]);
}

#[test]
fn vbb_like_agency_and_any_of_route_filters() {
    let mut t = tables();
    t.agencies = vec![
        agency("1", "Berliner Verkehrsbetriebe", "Europe/Berlin"),
        agency("2", "S-Bahn Berlin GmbH", "Europe/Berlin"),
        agency("3", "DB Regio AG", "Europe/Berlin"),
    ];
    t.routes = vec![
        route(
            "U1",
            Some("1"),
            Some("U1"),
            None,
            RouteType::Extended(ExtendedRouteType::Underground),
        ),
        route(
            "S1",
            Some("2"),
            Some("S1"),
            None,
            RouteType::Extended(ExtendedRouteType::SuburbanRailway),
        ),
        route("M10", Some("1"), Some("M10"), None, RouteType::Tramway),
        route("100", Some("1"), Some("100"), None, RouteType::Bus),
        route("RE1", Some("3"), Some("RE1"), None, RouteType::Rail),
    ];
    let sel = run(
        &t,
        vec![include(vec![
            agencies(&["Berliner Verkehrsbetriebe", "S-Bahn Berlin GmbH"]),
            any_of(vec![
                route_prefixed(RouteType::Subway, "U"),
                route_prefixed(RouteType::Rail, "S"),
            ]),
        ])],
    );
    assert_eq!(route_ids(&sel), ["S1", "U1"]);
    assert_eq!(agency_ids(&sel), ["1", "2"]);
}

// ---- cascade: routes -> trips -> stop_times -> stops / services ----

fn cascade_world() -> Tables {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![
        route("keep", Some("a"), Some("K1"), None, RouteType::Bus),
        route("drop", Some("a"), Some("D1"), None, RouteType::Bus),
    ];
    t.trips = vec![
        trip("t_keep", "keep", "svc_a"),
        trip("t_drop", "drop", "svc_b"),
    ];
    t.stop_times = vec![
        stop_time("t_keep", "s1", 1),
        stop_time("t_keep", "s2", 2),
        stop_time("t_drop", "s9", 1),
    ];
    t.stops = vec![stop("s1", None), stop("s2", None), stop("s9", None)];
    t.calendar = vec![calendar("svc_a"), calendar("svc_b")];
    t.calendar_dates = vec![
        calendar_date("svc_a", "20240110", 1),
        calendar_date("svc_b", "20240110", 1),
        calendar_date("svc_unreferenced", "20240110", 1),
    ];
    t
}

fn keep_prefix_k() -> Vec<GtfsFeedFilter> {
    vec![include(vec![route_prefixed(RouteType::Bus, "K")])]
}

#[test]
fn cascade_selects_trips_of_kept_routes() {
    let sel = run(&cascade_world(), keep_prefix_k());
    assert_eq!(trip_ids(&sel), ["t_keep"]);
}

#[test]
fn cascade_selects_stop_times_of_kept_trips() {
    let sel = run(&cascade_world(), keep_prefix_k());
    assert_eq!(sel.stop_times.len(), 2);
    assert!(sel.stop_times.iter().all(|st| st.trip_id == "t_keep"));
}

#[test]
fn cascade_selects_referenced_stops_only() {
    let sel = run(&cascade_world(), keep_prefix_k());
    assert_eq!(stop_ids(&sel), ["s1", "s2"]);
}

#[test]
fn cascade_selects_services_of_kept_trips() {
    let sel = run(&cascade_world(), keep_prefix_k());
    assert_eq!(calendar_service_ids(&sel), ["svc_a"]);
    assert_eq!(calendar_date_service_ids(&sel), ["svc_a"]);
}

fn parent_chain_world(stops: Vec<StopRow>) -> Tables {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "Europe/Berlin")];
    t.routes = vec![route("r1", Some("a"), Some("R1"), None, RouteType::Bus)];
    t.trips = vec![trip("t1", "r1", "svc")];
    t.stop_times = vec![stop_time("t1", "s1", 1)];
    t.stops = stops;
    t.calendar = vec![calendar("svc")];
    t
}

#[test]
fn parent_stations_are_pulled_in_transitively() {
    let t = parent_chain_world(vec![
        stop("s1", Some("p1")),
        stop("p1", Some("p2")),
        stop("p2", None),
        stop("unrelated", None),
    ]);
    let sel = run(&t, vec![include(vec![route_of(RouteType::Bus)])]);
    assert_eq!(stop_ids(&sel), ["p1", "p2", "s1"]);
}

#[test]
fn blank_parent_station_is_ignored() {
    let t = parent_chain_world(vec![stop("s1", Some(""))]);
    let sel = run(&t, vec![include(vec![route_of(RouteType::Bus)])]);
    assert_eq!(stop_ids(&sel), ["s1"]);
}

#[test]
fn parent_station_cycle_terminates() {
    let t = parent_chain_world(vec![stop("s1", Some("s2")), stop("s2", Some("s1"))]);
    let sel = run(&t, vec![include(vec![route_of(RouteType::Bus)])]);
    assert_eq!(stop_ids(&sel), ["s1", "s2"]);
}

// ---- agency timezone validation ----

#[test]
fn referenced_agency_timezone_mismatch_is_an_error() {
    let mut t = tables();
    t.agencies = vec![agency("a", "Agency A", "America/New_York")];
    t.routes = vec![route("u1", Some("a"), Some("U1"), None, RouteType::Subway)];
    let err = select_err(&t, &[include(vec![route_of(RouteType::Subway)])], BERLIN);
    assert!(err.contains("timezone mismatch"), "{err}");
    assert!(err.contains("America/New_York"), "{err}");
}

#[test]
fn unreferenced_agency_timezone_mismatch_is_ignored() {
    let mut t = tables();
    t.agencies = vec![
        agency("a", "Agency A", "Europe/Berlin"),
        agency("ghost", "Ghost", "America/New_York"),
    ];
    t.routes = vec![route("u1", Some("a"), Some("U1"), None, RouteType::Subway)];
    let sel = run(&t, vec![include(vec![route_of(RouteType::Subway)])]);
    assert_eq!(route_ids(&sel), ["u1"]);
    assert_eq!(agency_ids(&sel), ["a"]);
}

#[test]
fn blank_agency_id_still_triggers_timezone_check() {
    let mut t = tables();
    t.agencies = vec![agency("solo", "Solo", "America/New_York")];
    t.routes = vec![route("u1", None, Some("U1"), None, RouteType::Subway)];
    let err = select_err(&t, &[include(vec![route_of(RouteType::Subway)])], BERLIN);
    assert!(err.contains("timezone mismatch"), "{err}");
}

#[test]
fn selected_agencies_are_limited_to_referenced() {
    let mut t = tables();
    t.agencies = vec![
        agency("a", "Agency A", "Europe/Berlin"),
        agency("b", "Agency B", "Europe/Berlin"),
    ];
    t.routes = vec![
        route("ra", Some("a"), Some("RA"), None, RouteType::Bus),
        route("rb", Some("b"), Some("RB"), None, RouteType::Bus),
    ];
    let sel = run(&t, vec![include(vec![agencies(&["Agency A"])])]);
    assert_eq!(agency_ids(&sel), ["a"]);
}

// ---- misc edge cases ----

#[test]
fn empty_tables_yield_empty_selection() {
    let sel = run(&tables(), vec![]);
    assert!(route_ids(&sel).is_empty());
    assert!(trip_ids(&sel).is_empty());
    assert!(stop_ids(&sel).is_empty());
    assert!(agency_ids(&sel).is_empty());
}
