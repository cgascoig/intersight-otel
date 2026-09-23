use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{bail, Result};
use generic_poller::Aggregator;
use intersight_api::Client;
use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, KeyValue};
use tokio::{sync::mpsc::Sender, task::JoinHandle, time};

const ENRICH_TIMEOUT_SECS: u64 = 60;

use crate::attribute_enricher::AttributeEnricher;
use crate::config::{OTelAttributeProvider, PollerConfig, TSPollerConfig};

mod generic_poller;
mod timeseries_poller;

#[derive(Debug, Clone)]
pub struct IntersightMetric {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub value: f64,
    pub timestamp_offset: u64,
    pub start_time: SystemTime,
    pub time: SystemTime,
}

impl IntersightMetric {
    pub fn new(
        name: &str,
        value: f64,
        attributes: Option<BTreeMap<String, String>>,
        start_time: SystemTime,
        time: SystemTime,
    ) -> IntersightMetric {
        IntersightMetric {
            name: name.to_string(),
            attributes: attributes.unwrap_or_default(),
            value,
            timestamp_offset: 0,
            start_time,
            time,
        }
    }
}

#[derive(Default)]
pub struct IntersightResourceMetrics {
    pub attributes: Vec<KeyValue>,
    pub metrics: Vec<IntersightMetric>,
    pub start_time: Option<SystemTime>,
}

pub type IntersightMetricBatch = Vec<IntersightResourceMetrics>;

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
    tx: Sender<IntersightMetricBatch>,
    client: &Client,
    config: &PollerConfig,
    enrichers: Vec<Arc<AttributeEnricher>>,
) -> Result<JoinHandle<()>> {
    let client = (*client).clone();
    let config = (*config).clone();
    let interval = config.interval();
    let query = config.api_query.clone();
    let method = config.api_method.clone();
    let body = config.api_body.clone();

    let aggregator = get_aggregator_for_config(&config)?;

    let handle = tokio::spawn(async move {
        let mut interval = time::interval(time::Duration::from_secs(interval));

        loop {
            let start_time = SystemTime::now();
            interval.tick().await;

            let poll_result =
                generic_poller::poll(&client, &query, &method, &body, aggregator.as_ref()).await;

            if let Ok(mut r) = poll_result {
                add_otel_attributes(&mut r, &config);
                if !enrichers.is_empty() {
                    let enrich_result =
                        time::timeout(time::Duration::from_secs(ENRICH_TIMEOUT_SECS), async {
                            for enricher in &enrichers {
                                enricher.enrich_batch(&mut r).await;
                            }
                        })
                        .await;
                    if enrich_result.is_err() {
                        warn!(
                            "Poller '{}': enrichment timed out after {}s, sending batch un-enriched",
                            config.name, ENRICH_TIMEOUT_SECS
                        );
                    }
                }
                let metric_count: usize = r.iter().map(|rm| rm.metrics.len()).sum();
                let resource_count = r.len();
                if resource_count == 0 {
                    warn!(
                        "Poller '{}': poll returned empty batch this tick",
                        config.name
                    );
                } else {
                    debug!(
                        "Poller '{}': sending {} resources, {} metrics",
                        config.name, resource_count, metric_count
                    );
                }
                add_start_time(&mut r, start_time);
                if let Err(err) = tx.send(r).await {
                    error!("metrics receiver thread dropped: {}", err);
                }
            } else if let Err(err) = poll_result {
                error!("error while polling Intersight: {}", err);
            }
        }
    });

    Ok(handle)
}

pub fn start_intersight_tspoller(
    tx: Sender<IntersightMetricBatch>,
    client: &Client,
    config: &TSPollerConfig,
    enrichers: Vec<Arc<AttributeEnricher>>,
) -> Result<JoinHandle<()>> {
    let client = (*client).clone();
    let config = (*config).clone();

    let handle = tokio::spawn(async move {
        let mut interval = time::interval(time::Duration::from_secs(config.interval()));

        loop {
            let start_time = SystemTime::now();
            interval.tick().await;

            let poll_result = timeseries_poller::poll(&client, &config).await;

            if let Ok(mut r) = poll_result {
                add_otel_attributes(&mut r, &config);
                if !enrichers.is_empty() {
                    let enrich_result =
                        time::timeout(time::Duration::from_secs(ENRICH_TIMEOUT_SECS), async {
                            for enricher in &enrichers {
                                enricher.enrich_batch(&mut r).await;
                            }
                        })
                        .await;
                    if enrich_result.is_err() {
                        warn!(
                            "TSPoller '{}': enrichment timed out after {}s, sending batch un-enriched",
                            config.name, ENRICH_TIMEOUT_SECS
                        );
                    }
                }
                let metric_count: usize = r.iter().map(|rm| rm.metrics.len()).sum();
                let resource_count = r.len();
                if resource_count == 0 {
                    warn!(
                        "TSPoller '{}': poll returned empty batch this tick",
                        config.name
                    );
                } else {
                    debug!(
                        "TSPoller '{}': sending {} resources, {} metrics",
                        config.name, resource_count, metric_count
                    );
                }
                add_start_time(&mut r, start_time);
                if let Err(err) = tx.send(r).await {
                    error!("metrics receiver thread dropped: {}", err);
                }
            } else if let Err(err) = poll_result {
                error!("error while polling Intersight: {}", err);
            }
        }
    });

    Ok(handle)
}

fn add_otel_attributes(batch: &mut IntersightMetricBatch, config: &impl OTelAttributeProvider) {
    for metrics in batch {
        for (k, v) in config.otel_attributes() {
            metrics.attributes.push(KeyValue {
                key: k,
                value: Some(AnyValue {
                    value: Some(any_value::Value::StringValue(v)),
                }),
                key_strindex: 0,
            })
        }
    }
}

fn add_start_time(batch: &mut IntersightMetricBatch, start_time: SystemTime) {
    for metrics in batch {
        metrics.start_time = Some(start_time);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn poller_config(aggregator: &str) -> PollerConfig {
        serde_json::from_value(json!({
            "api_query": "api/v1/example",
            "aggregator": aggregator,
            "name": "example.metric",
            "otel_attributes": {
                "environment": "test",
                "site": "lab"
            }
        }))
        .unwrap()
    }

    #[test]
    fn selects_result_count_aggregator() {
        let aggregator = get_aggregator_for_config(&poller_config("result_count")).unwrap();
        let batch = aggregator.aggregate(json!({ "Count": 7 }));

        assert_eq!(batch[0].metrics[0].name, "example.metric");
        assert_eq!(batch[0].metrics[0].value, 7.0);
    }

    #[test]
    fn selects_count_results_aggregator() {
        let aggregator = get_aggregator_for_config(&poller_config("count_results")).unwrap();
        let batch = aggregator.aggregate(json!({ "Results": [{}, {}] }));

        assert_eq!(batch[0].metrics[0].name, "example.metric");
        assert_eq!(batch[0].metrics[0].value, 2.0);
    }

    #[test]
    fn rejects_unknown_aggregator() {
        let error = get_aggregator_for_config(&poller_config("unknown"))
            .err()
            .unwrap();

        assert_eq!(error.to_string(), "Invalid aggregator unknown");
    }

    #[test]
    fn adds_static_attributes_to_every_resource() {
        let mut batch = vec![
            IntersightResourceMetrics::default(),
            IntersightResourceMetrics::default(),
        ];

        add_otel_attributes(&mut batch, &poller_config("result_count"));

        for resource in batch {
            assert_eq!(resource.attributes.len(), 2);
            let attributes: BTreeMap<_, _> = resource
                .attributes
                .into_iter()
                .map(|attribute| {
                    let value = match attribute.value.unwrap().value.unwrap() {
                        any_value::Value::StringValue(value) => value,
                        value => panic!("unexpected attribute value: {value:?}"),
                    };
                    (attribute.key, value)
                })
                .collect();
            assert_eq!(
                attributes.get("environment").map(String::as_str),
                Some("test")
            );
            assert_eq!(attributes.get("site").map(String::as_str), Some("lab"));
        }
    }

    #[test]
    fn adds_start_time_to_every_resource() {
        let mut batch = vec![
            IntersightResourceMetrics::default(),
            IntersightResourceMetrics::default(),
        ];
        let start_time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(123);

        add_start_time(&mut batch, start_time);

        assert!(batch
            .iter()
            .all(|resource| resource.start_time == Some(start_time)));
    }
}
