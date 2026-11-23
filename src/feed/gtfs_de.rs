use std::time::Duration;

use gtfs_structures::RouteType;

use crate::feed::{
    Feed, GtfsFeed, GtfsFeedFilter, GtfsFeedFilterAction, GtfsFeedFilterCriteria,
    GtfsFeedFilterRoute, GtfsFeedScope, GtfsRtFeed, RemoteSource, minutes,
};

pub const FEED: Feed = Feed {
    schedule: Some(GtfsFeed {
        ident: "gtfs_de",
        source: RemoteSource::at("https://download.gtfs.de/germany/free/latest.zip") // CC BY 4.0
            .timeout(Duration::from_secs(60)),
        scopes: &[
            GtfsFeedScope {
                ident: "berlin_s_u",
                filters: &[GtfsFeedFilter {
                    // Berlin S-Bahn (DB) and U-Bahn (BVG) routes only
                    action: GtfsFeedFilterAction::Include,
                    criteria: &[
                        GtfsFeedFilterCriteria::Agencies(&[
                            "Berliner Verkehrsbetriebe",
                            "S Bahn Berlin GmbH",
                        ]),
                        GtfsFeedFilterCriteria::AnyOf(&[
                            GtfsFeedFilterCriteria::RouteFilter(GtfsFeedFilterRoute {
                                typ: RouteType::Subway,
                                prefix: Some("U"),
                            }),
                            GtfsFeedFilterCriteria::RouteFilter(GtfsFeedFilterRoute {
                                typ: RouteType::Rail,
                                prefix: Some("S"),
                            }),
                        ]),
                    ],
                }],
                max_early: minutes(15),
                max_late: minutes(60),
            },
            GtfsFeedScope {
                ident: "germany_db_fv",
                filters: &[GtfsFeedFilter {
                    // DB long-distance rail: ICE, IC, EC
                    action: GtfsFeedFilterAction::Include,
                    criteria: &[
                        GtfsFeedFilterCriteria::Agencies(&["DB Fernverkehr AG"]),
                        GtfsFeedFilterCriteria::RouteFilter(GtfsFeedFilterRoute {
                            typ: RouteType::Rail,
                            prefix: None,
                        }),
                    ],
                }],
                max_early: minutes(30),
                max_late: minutes(4 * 60),
            },
        ],
    }),
    realtime: &[GtfsRtFeed {
        ident: "gtfs_de-rt",
        source: RemoteSource::at("https://realtime.gtfs.de/realtime-free.pb") // CC BY-SA 4.0
            .timeout(Duration::from_secs(30))
            .max_retries(1),
    }],
    timezone: chrono_tz::Europe::Berlin,
    plugin: None,
};
