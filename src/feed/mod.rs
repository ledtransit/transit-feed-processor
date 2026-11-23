mod delfi;
mod gtfs_de;
mod vbb;

use chrono_tz::Tz;
use gtfs_structures::RouteType;

use crate::plugin::FeedProcPlugin;
use std::time::Duration;

pub struct Feed {
    pub schedule: Option<GtfsFeed>,      // GTFS static feed
    pub realtime: &'static [GtfsRtFeed], // GTFS-RT realtime feeds
    pub timezone: Tz,                    // Must match all filtered agencies' agency_timezone
    pub plugin: Option<FeedProcPlugin>,  // Optional plugin for realtime data enrichment
}

pub struct GtfsFeed {
    pub ident: &'static str,
    pub source: RemoteSource,
    pub scopes: &'static [GtfsFeedScope], // Filter static feed routes into named scopes e.g. only metro+tram by agency A
}

pub struct GtfsFeedScope {
    pub ident: &'static str,
    pub filters: &'static [GtfsFeedFilter], // At least one filter must match (OR-combined)
    pub max_early: Duration, // How far *ahead* of schedule a vehicle of this scope may plausibly run
    pub max_late: Duration,  // How far *behind* schedule a vehicle of this scope may plausibly run
}

pub struct GtfsFeedFilter {
    pub action: GtfsFeedFilterAction, // Positive or negative filter
    pub criteria: &'static [GtfsFeedFilterCriteria], // All criteria must match for the filter to match
}

pub enum GtfsFeedFilterAction {
    Include,
    #[allow(dead_code)]
    Exclude,
}

pub enum GtfsFeedFilterCriteria {
    Agencies(&'static [&'static str]), // Agency matches agency_name
    RouteFilter(GtfsFeedFilterRoute), // Route matches route_type and optional route_short_name prefix
    AnyOf(&'static [GtfsFeedFilterCriteria]), // OR-combined criteria
}

pub struct GtfsFeedFilterRoute {
    pub typ: RouteType,
    pub prefix: Option<&'static str>,
}

pub struct GtfsRtFeed {
    pub ident: &'static str,
    pub source: RemoteSource,
}

pub const fn minutes(m: u64) -> Duration {
    Duration::from_secs(m * 60)
}

pub struct RemoteSource {
    pub url: &'static str,
    pub timeout: Duration,
    pub max_retries: u32,
    pub query_auth: Option<QueryAuth>, // Secret query parameter appended to the url
    pub client_cert: Option<ClientCert>, // TLS client certificate sent with the request
}

/// A query parameter whose value is read from an env var at runtime.
pub struct QueryAuth {
    pub param: &'static str,
    pub env: &'static str,
}

/// A password protected PKCS#12 client certificate, relative to the working
/// directory, whose password is read from an env var at runtime.
pub struct ClientCert {
    pub path: &'static str,
    pub password_env: &'static str,
}

impl RemoteSource {
    pub const fn at(url: &'static str) -> Self {
        Self {
            url,
            timeout: Duration::from_secs(10),
            max_retries: 2,
            query_auth: None,
            client_cert: None,
        }
    }

    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub const fn max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub const fn query_auth(mut self, param: &'static str, env: &'static str) -> Self {
        self.query_auth = Some(QueryAuth { param, env });
        self
    }

    pub const fn client_cert(mut self, path: &'static str, password_env: &'static str) -> Self {
        self.client_cert = Some(ClientCert { path, password_env });
        self
    }

    pub const fn needs_auth(&self) -> bool {
        self.query_auth.is_some() || self.client_cert.is_some()
    }
}

pub static FEEDS: &[Feed] = &[vbb::FEED, delfi::FEED, gtfs_de::FEED];
