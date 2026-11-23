use anyhow::Result;
use sqlx::PgPool;

const NOTIFY_SQL: &str = "SELECT pg_notify('transit_realtime_updated', $1)";

/// Notify Postgres that the given rt sources finished polling. The payload is
/// their comma-separated idents, e.g. `vbb-rt`.
pub async fn notify_realtime_updated(pool: &PgPool, rt_sources: &[&str]) -> Result<()> {
    sqlx::query(NOTIFY_SQL)
        .bind(rt_sources.join(","))
        .execute(pool)
        .await?;
    Ok(())
}
