use std::time::Duration;

use gtfs_structures::RouteType;

use crate::feed::{
    Feed, GtfsFeed, GtfsFeedFilter, GtfsFeedFilterAction, GtfsFeedFilterCriteria,
    GtfsFeedFilterRoute, GtfsFeedScope, GtfsRtFeed, RemoteSource, minutes,
};
use crate::plugin::plugin_feed;

pub const FEED: Feed = Feed {
    schedule: Some(GtfsFeed {
        ident: "vbb",
        source: RemoteSource::at("https://www.vbb.de/vbbgtfs").timeout(Duration::from_secs(30)), // CC BY 4.0
        scopes: &[GtfsFeedScope {
            ident: "berlin_s_u",
            filters: &[GtfsFeedFilter {
                // Filter for S-Bahn (DB) and U-Bahn (BVG) routes only
                action: GtfsFeedFilterAction::Include,
                criteria: &[
                    GtfsFeedFilterCriteria::Agencies(&[
                        "Berliner Verkehrsbetriebe",
                        "S-Bahn Berlin GmbH",
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
            max_early: minutes(5),
            max_late: minutes(60),
        }],
    }),
    realtime: &[GtfsRtFeed {
        ident: "vbb-rt",
        source: RemoteSource::at("https://production.gtfsrt.vbb.de/data"), // CC BY 4.0
    }],
    timezone: chrono_tz::Europe::Berlin,
    plugin: plugin_feed!(vbb),
};
