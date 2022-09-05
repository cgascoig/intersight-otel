use anyhow::{bail, Result};
use generic_poller::Aggregator;
use intersight_api::Client;
use opentelemetry::Value;
use tokio::{sync::mpsc::Sender, task::JoinHandle, time};

use crate::config::PollerConfig;

mod generic_poller;

#[derive(Debug)]
pub struct IntersightMetric {
    pub name: String,
    pub value: Value,
}

fn get_aggregator_for_config(config: &PollerConfig) -> Result<Box<dyn Aggregator + Sync + Send>> {
    match config.aggregator.as_str() {
        "result_count" => Ok(Box::new(generic_poller::ResultCountAggregator::new(
            config.name.clone(),
        ))),
        "count_results" => Ok(Box::new(generic_poller::ResultCountingAggregator::new(
            config.name.clone(),
        ))),
        _ => bail!(format!("Invalid aggregator {}", config.aggregator)),
    }
}

pub fn start_intersight_poller(
    tx: Sender<IntersightMetric>,
    client: &Client,
    config: &PollerConfig,
) -> Result<JoinHandle<()>> {
    let client = (*client).clone();
    let interval = config.interval();
    let query = config.api_query.clone();

    let aggregator = get_aggregator_for_config(&config)?;

    let handle = tokio::spawn(async move {
        let mut interval = time::interval(time::Duration::from_secs(interval));

        loop {
            interval.tick().await;

            let poll_result = generic_poller::poll(&client, &query, aggregator.as_ref()).await;

            if let Ok(r) = poll_result {
                for metric in r {
                    tx.send(metric).await.unwrap();
                }
            } else if let Err(err) = poll_result {
                error!("error while polling Intersight: {}", err);
            }
        }
    });

    Ok(handle)
}
