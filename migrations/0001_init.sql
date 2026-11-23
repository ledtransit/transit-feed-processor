-- Static GTFS mirror + realtime overlay.
--
-- A schedule feed is sliced into named scopes (e.g. "berlin_s_u",
-- "germany_long_distance"), each with its own filter and realtime
-- delay-plausibility window. `feed` + `scope` together key every row, so
-- consumers can select exactly one slice, e.g.
-- feed=vbb + scope=berlin_s_u + rt_source=vbb-rt.

create extension if not exists btree_gist;

-- Agencies referenced by the static mirror, needed to match realtime sources
-- that report an operator name but no gtfs agency_id.
create table agencies (
    feed            text not null,
    scope           text not null,
    agency_id       text not null,
    agency_name     text not null,
    agency_timezone text not null,
    primary key (feed, scope, agency_id)
);

create table routes (
    feed                text    not null,
    scope               text    not null,
    route_id            text    not null,
    agency_id           text,
    route_short_name    text,
    route_long_name     text,
    route_desc          text,
    route_type          smallint not null,
    route_url           text,
    route_color         text    not null default 'FFFFFF',
    route_text_color    text    not null default '000000',
    route_sort_order    integer,
    continuous_pickup   smallint,
    continuous_drop_off smallint,
    network_id          text,
    primary key (feed, scope, route_id)
);

create table stops (
    feed                text not null,
    scope               text not null,
    stop_id             text not null,
    stop_code           text,
    stop_name           text,
    tts_stop_name       text,
    stop_desc           text,
    stop_lat            double precision,
    stop_lon            double precision,
    zone_id             text,
    stop_url            text,
    location_type       smallint,
    parent_station      text,
    stop_timezone       text,
    wheelchair_boarding smallint,
    level_id            text,
    platform_code       text,
    primary key (feed, scope, stop_id)
);

create index stops_feed_scope_parent_station on stops (feed, scope, parent_station);

create table calendar (
    feed       text    not null,
    scope      text    not null,
    service_id text    not null,
    monday     boolean not null,
    tuesday    boolean not null,
    wednesday  boolean not null,
    thursday   boolean not null,
    friday     boolean not null,
    saturday   boolean not null,
    sunday     boolean not null,
    start_date date    not null,
    end_date   date    not null,
    primary key (feed, scope, service_id)
);

create table calendar_dates (
    feed           text     not null,
    scope          text     not null,
    service_id     text     not null,
    date           date     not null,
    exception_type smallint not null,
    primary key (feed, scope, service_id, date)
);

create table trips (
    feed                  text not null,
    scope                 text not null,
    trip_id               text not null,
    route_id              text not null,
    service_id            text not null,
    trip_headsign         text,
    trip_short_name       text,
    direction_id          smallint,
    block_id              text,
    shape_id              text,
    wheelchair_accessible smallint,
    bikes_allowed         smallint,
    -- seconds after service-date midnight (may exceed 86400); see trips_active_span
    start_time            integer,
    end_time              integer,
    primary key (feed, scope, trip_id),
    foreign key (feed, scope, route_id) references routes (feed, scope, route_id) on delete cascade
);

create index trips_feed_scope_route_id on trips (feed, scope, route_id);
create index trips_feed_scope_service_id on trips (feed, scope, service_id);

-- Drives the "currently running" lookup with a single GiST range-overlap probe
-- instead of aggregating stop_times per query.
create index trips_active_span
    on trips using gist (feed, scope, int4range(start_time, end_time, '[]'));

create table stop_times (
    feed                text    not null,
    scope               text    not null,
    trip_id             text    not null,
    stop_sequence       integer not null,
    stop_id             text    not null,
    -- seconds after midnight; may exceed 86400 for trips past 24:00
    arrival_time        integer,
    departure_time      integer,
    stop_headsign       text,
    pickup_type         smallint,
    drop_off_type       smallint,
    continuous_pickup   smallint,
    continuous_drop_off smallint,
    shape_dist_traveled double precision,
    timepoint           smallint,
    primary key (feed, scope, trip_id, stop_sequence),
    foreign key (feed, scope, trip_id) references trips (feed, scope, trip_id) on delete cascade,
    foreign key (feed, scope, stop_id) references stops (feed, scope, stop_id) on delete cascade
);

create index stop_times_feed_scope_stop_id on stop_times (feed, scope, stop_id);

-- Realtime overlay. One snapshot per (feed, scope, rt_source), fully
-- rewritten each poll, so the tables always describe "what is running right
-- now". `rt_source` is a realtime provider — a static feed may have several,
-- fetched independently. Consumers filter on
-- (feed, scope, rt_source) and optionally a service_date / time window, and
-- join stops via (feed, scope, rt_source, trip_id, service_date).
--
-- Last-known-value overlay: gtfs-rt feeds are full snapshots, but real feeds
-- drop trips/stops transiently. A row survives until its trip leaves the
-- candidate window, and updated_at / rt_updated_at track when it was last
-- touched vs. last actually backed by live data (null rt_updated_at =
-- schedule-only). Sticky facts: trip schedule_relationship canceled(3)/
-- deleted(7) and stop schedule_relationship skipped(1) persist once seen.

create table rt_trip_update (
    feed                  text        not null,
    scope                 text        not null,
    rt_source             text        not null,
    trip_id               text        not null,
    service_date          date        not null,
    route_id              text,
    -- scheduled span, seconds after service_date midnight (may exceed 86400)
    start_time            integer,
    end_time              integer,
    -- TripDescriptor.ScheduleRelationship: 0 scheduled, 1 added, 2 unscheduled,
    -- 3 canceled, 5 replacement, 6 duplicated, 7 deleted, 8 new
    schedule_relationship smallint    not null default 0,
    -- true when the gtfs-rt feed carried a TripUpdate for this trip; false means
    -- it is a schedule-only candidate (assumed running on time, no stop rows)
    has_realtime          boolean     not null,
    -- trip-level TripUpdate.delay, seconds (+late / -early)
    trip_delay            integer,
    -- TripUpdate.timestamp, POSIX seconds
    rt_timestamp          bigint,
    vehicle_id            text,
    vehicle_label         text,
    updated_at            timestamptz not null default now(),
    rt_updated_at         timestamptz,
    primary key (feed, scope, rt_source, trip_id, service_date)
);

create index rt_trip_update_window
    on rt_trip_update (feed, scope, rt_source, service_date, start_time, end_time);

create table rt_stop_time_update (
    feed                  text        not null,
    scope                 text        not null,
    rt_source             text        not null,
    trip_id               text        not null,
    service_date          date        not null,
    stop_sequence         integer     not null,
    stop_id               text,
    -- StopTimeUpdate.ScheduleRelationship: 0 scheduled, 1 skipped, 2 no_data,
    -- 3 unscheduled
    schedule_relationship smallint    not null default 0,
    -- whether the source reported on this stop in its latest poll, rather than
    -- the row being materialised from the trip's schedule and carried delays;
    -- only a reported stop may change its schedule_relationship
    reported              boolean     not null default true,
    -- effective delay after forward-fill along the trip, seconds
    -- (null = unknown, e.g. past a no_data stop)
    arrival_delay         integer,
    departure_delay       integer,
    -- raw StopTimeEvent.time from the feed, POSIX seconds
    arrival_time          bigint,
    departure_time        bigint,
    updated_at            timestamptz not null default now(),
    rt_updated_at         timestamptz,
    primary key (feed, scope, rt_source, trip_id, service_date, stop_sequence),
    foreign key (feed, scope, rt_source, trip_id, service_date)
        references rt_trip_update (feed, scope, rt_source, trip_id, service_date) on delete cascade
);
