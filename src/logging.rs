use log::LevelFilter;

// Exclude some verbose modules from logging at below warning level
const EXCLUDE_MODULES: &[&str] = &[
    "h2",
    "hyper",
    "hyper_util",
    "reqwest",
    "rustls",
    "tokio_util",
    "mio",
    "want",
    "sqlx",
    "sqlx_core",
    "tokio_postgres",
    "tokio_cron_scheduler",
];

pub fn init() {
    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    for module in EXCLUDE_MODULES {
        logger.filter_module(module, LevelFilter::Warn);
    }
    logger.init();
}
