use dotenvy::dotenv;
use log::info;

use transit_feed_processor::{cron, db, logging, net};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    logging::init();

    // Fail fast if the outbound identity is missing or still the placeholder.
    net::user_agent()?;

    // Connect to postgres db and migrate
    let pool = db::connect().await?;

    // Start the scheduler for recurring jobs (static refresh and realtime poll)
    let _scheduler = cron::start(pool).await?;
    info!("scheduler running");

    tokio::signal::ctrl_c().await?;
    info!("shutdown");
    Ok(())
}
