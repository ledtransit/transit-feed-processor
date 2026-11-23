pub mod copy;
pub mod notify;

use anyhow::{Context, Result};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// Connect to postgres db and run migrations
pub async fn connect() -> Result<PgPool> {
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .context("connecting to postgres")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running migrations")?;

    Ok(pool)
}
