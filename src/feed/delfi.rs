use std::time::Duration;

use gtfs_structures::RouteType;

use crate::feed::{
    Feed, GtfsFeed, GtfsFeedFilter, GtfsFeedFilterAction, GtfsFeedFilterCriteria,
    GtfsFeedFilterRoute, GtfsFeedScope, GtfsRtFeed, RemoteSource, minutes,
};

pub const FEED: Feed = Feed {
    schedule: Some(GtfsFeed {
        ident: "delfi",
        source: RemoteSource::at("https://data.public-transport.earth/gtfs/de") // CC BY 4.0
            .timeout(Duration::from_secs(60)),
        scopes: &[GtfsFeedScope {
            ident: "berlin_s_u",
            filters: &[GtfsFeedFilter {
                // Berlin S-Bahn (DB) and U-Bahn (BVG) routes only
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
            max_early: minutes(15),
            max_late: minutes(60),
        }],
    }),
    realtime: &[GtfsRtFeed {
        ident: "delfi-rt",
        source: RemoteSource::at(
            "https://mobilithek.info:8443/mobilithek/api/v1.0/container/subscription",
        ) // CC BY-SA 4.0
        .timeout(Duration::from_secs(15))
        .query_auth("subscriptionID", "MOBILITHEK_DELFI_GTFS_RT_SUBSCRIPTION_ID")
        .client_cert("cert/mobilithek.p12", "MOBILITHEK_M2M_CERT_PASSWORD"),
    }],
    timezone: chrono_tz::Europe::Berlin,
    plugin: None,
};
