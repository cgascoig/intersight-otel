use anyhow::{Context, Result};
use opentelemetry::metrics;
use opentelemetry::sdk::export;
use opentelemetry::sdk::metrics::PushController;

#[macro_use]
extern crate log;

mod config;
mod intersight_poller;
mod metric_merger;

const KEY_ID: &str = include_str!("../../creds/intersight-cgascoig-20200423.keyid.txt");
const SECRET_KEY: &[u8] = include_bytes!("../../creds/intersight-cgascoig-20200423.pem");

// const KEY_ID: &str = include_str!("../../creds/intersight-cgascoig-v3-20220329.keyid.txt");
// const SECRET_KEY: &[u8] = include_bytes!("../../creds/intersight-cgascoig-v3-20220329.pem");

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("starting up");
    info!("Using key_id {}", KEY_ID);

    let config = config::GlobalConfig::new().context("Unable to load config")?;
    info!("Config: {:?}", config);

    let _ctrl = init_metrics_stdout()?;

    let client = intersight_api::Client::from_key_bytes(KEY_ID, SECRET_KEY, None)?;

    let (metric_chan_tx, metric_chan_rx) = tokio::sync::mpsc::channel(32);

    let merge_handle = metric_merger::merge_metrics(metric_chan_rx);

    if let Some(poller_configs) = config.pollers {
        for poller_config in poller_configs {
            intersight_poller::start_intersight_poller(
                metric_chan_tx.clone(),
                &client,
                &poller_config,
            )?;
        }
    }

    merge_handle.await?;

    Ok(())
}

// fn init_metrics_otlp() -> metrics::Result<PushController> {
//     let export_config = ExportConfig {
//         endpoint: "http://localhost:4317".to_string(),
//         ..ExportConfig::default()
//     };
//     opentelemetry_otlp::new_pipeline()
//         .metrics(tokio::spawn, opentelemetry::util::tokio_interval_stream)
//         .with_exporter(
//             opentelemetry_otlp::new_exporter()
//                 .tonic()
//                 .with_export_config(export_config),
//         )
//         .build()
// }

use futures_util::{Stream, StreamExt as _};
use std::time::Duration;

// Skip first immediate tick from tokio, not needed for async_std.
fn delayed_interval(duration: Duration) -> impl Stream<Item = tokio::time::Instant> {
    opentelemetry::util::tokio_interval_stream(duration).skip(1)
}

fn init_metrics_stdout() -> metrics::Result<PushController> {
    let exporter = export::metrics::stdout(tokio::spawn, delayed_interval)
        .with_period(Duration::from_secs(5))
        .init();

    Ok(exporter)
}
