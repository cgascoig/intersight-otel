use anyhow::{Context, Result};
use std::fs;

#[macro_use]
extern crate log;

mod config;
mod intersight_poller;
mod metric_merger;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("starting up");

    let config = config::GlobalConfig::new().context("Unable to load config")?;
    info!(
        "Using key_id {} and key_file {}",
        config.key_id, config.key_file
    );

    let key_bytes = fs::read(config.key_file)?;
    let client = intersight_api::Client::from_key_bytes(&config.key_id, &key_bytes, None)?;

    // Create a multi-producer single-consumer channel for poller tasks to send metrics to the metric_merger task
    let (metric_chan_tx, metric_chan_rx) = tokio::sync::mpsc::channel(32);

    let merge_handle = metric_merger::start_metric_merger(metric_chan_rx);

    // Start all the pollers based on the config file(s)
    if let Some(poller_configs) = config.pollers {
        for poller_config in poller_configs {
            intersight_poller::start_intersight_poller(
                metric_chan_tx.clone(),
                &client,
                &poller_config,
            )?;
        }
    }

    // Keep running until the metric_merger finishes (i.e. never)
    merge_handle.await?;

    Ok(())
}
