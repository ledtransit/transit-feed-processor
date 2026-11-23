use std::time::Duration;

use anyhow::Result;
use log::error;
use sqlx::PgPool;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::timing::timed_phase;
use crate::{realtime, schedule};

/// Register the recurring jobs: static feeds now and every 30 minutes,
/// realtime feeds every 30 seconds.
pub async fn start(pool: PgPool) -> Result<JobScheduler> {
    let scheduler = JobScheduler::new().await?;

    let now_pool = pool.clone();
    scheduler
        .add(Job::new_one_shot_async(
            Duration::ZERO,
            move |_id, _sched| {
                let pool = now_pool.clone();
                Box::pin(async move { timed_phase("static refresh", static_refresh(&pool)).await })
            },
        )?)
        .await?;

    let static_pool = pool.clone();
    scheduler
        .add(Job::new_async("0 0/30 * * * *", move |_id, _sched| {
            let pool = static_pool.clone();
            Box::pin(async move { timed_phase("static refresh", static_refresh(&pool)).await })
        })?)
        .await?;

    scheduler
        .add(Job::new_async("*/30 * * * * *", move |_id, _sched| {
            let pool = pool.clone();
            Box::pin(async move { timed_phase("realtime poll", realtime_poll(&pool)).await })
        })?)
        .await?;

    scheduler.start().await?;
    Ok(scheduler)
}

async fn static_refresh(pool: &PgPool) {
    if let Err(e) = schedule::run_all_static(pool).await {
        error!("static refresh failed: {e:#}");
    }
}

async fn realtime_poll(pool: &PgPool) {
    if let Err(e) = realtime::run_all_realtime(pool).await {
        error!("realtime poll failed: {e:#}");
    }
}
