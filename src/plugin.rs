//! Optional out-of-tree extension point for additional realtime sources.
//!
//! With the `plugin` feature, the plugin's sources are compiled in from a
//! sibling `proc-plugin/` directory next to this crate (not part of this
//! repository). Without it, which is the default, a no-op stub is used and
//! every feed's `plugin` is `None`.

#[cfg(feature = "plugin")]
#[path = "../../proc-plugin/src/mod.rs"]
pub(crate) mod imp;

// Plugin unit tests live in `proc-plugin/tests/` and only compile with the
// feature enabled; run them with `cargo test --features plugin`.
#[cfg(all(test, feature = "plugin"))]
#[path = "../../proc-plugin/tests/mod.rs"]
mod tests;

#[cfg(feature = "plugin")]
pub(crate) use imp::FeedProcPlugin;
#[cfg(feature = "plugin")]
pub(crate) use imp::realtime::run as run_realtime;

#[cfg(not(feature = "plugin"))]
pub(crate) use stub::FeedProcPlugin;

#[cfg(not(feature = "plugin"))]
mod stub {
    pub enum FeedProcPlugin {}

    impl FeedProcPlugin {
        pub fn ident(&self) -> &'static str {
            match *self {}
        }
    }
}

#[cfg(not(feature = "plugin"))]
pub(crate) async fn run_realtime(
    _feed: &'static crate::feed::Feed,
    _scoped: &crate::realtime::candidates::FeedCandidates,
    _pool: &sqlx::PgPool,
) {
}

#[cfg(feature = "plugin")]
macro_rules! plugin_feed {
    ($feed:ident) => {
        ::core::option::Option::Some($crate::plugin::imp::feeds::$feed::PLUGIN)
    };
}

#[cfg(not(feature = "plugin"))]
macro_rules! plugin_feed {
    ($feed:ident) => {
        ::core::option::Option::None
    };
}

pub(crate) use plugin_feed;
