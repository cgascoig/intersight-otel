use std::collections::BTreeMap;
use std::time::SystemTime;

use anyhow::{bail, Result};
use generic_poller::Aggregator;
use intersight_api::Client;
// use opentelemetry_api::{global, metrics::Meter, KeyValue, Value};
use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, KeyValue};
use tokio::{sync::mpsc::Sender, task::JoinHandle, time};

use crate::config::{OTelAttributeProvider, PollerConfig, TSPollerConfig};

mod generic_poller;
mod timeseries_poller;

#[derive(Debug, Clone)]
pub struct IntersightMetric {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub value: f64,
    pub timestamp_offset: u64,
}

impl IntersightMetric {
    pub fn new(
        name: &str,
        value: f64,
        attributes: Option<BTreeMap<String, String>>,
    ) -> IntersightMetric {
        IntersightMetric {
            name: name.to_string(),
            attributes: attributes.unwrap_or_default(),
            value,
            timestamp_offset: 0,
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
                add_start_time(&mut r, start_time);
                if let Err(err) = tx.send(r).await {
                    error!("metrics receiver thread dropped");
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
            })
        }
    }
}

fn add_start_time(batch: &mut IntersightMetricBatch, start_time: SystemTime) {
    for metrics in batch {
        metrics.start_time = Some(start_time);
    }
}

// const METER_NAME: &str = "intersight-otel";

// static mut meter_store: Vec<Meter> = vec![];

// pub fn start_new_intersight_poller(client: &Client, config: &PollerConfig) -> Result<Meter> {
//     let name = config.name.clone();

//     info!("Starting new intersight poller {}", name);
//     let meter = global::meter_with_version(
//         METER_NAME,
//         Some(""),
//         Some(""),
//         Some(vec![KeyValue::new("key", "value")]),
//     );

//     let client = (*client).clone();
//     let config = (*config).clone();
//     let query = config.api_query.clone();
//     let method = config.api_method.clone();
//     let body = config.api_body.clone();

//     let aggregator = get_aggregator_for_config(&config)?;

//     let guage = meter.f64_observable_gauge(name.clone()).init();

//     meter.register_callback(&[guage.as_any()], move |obs| {
//         debug!("Got observation callback for {}", name);

//         let poll_result =
//             generic_poller::poll_sync(&client, &query, &method, &body, aggregator.as_ref());

//         if let Ok(mut r) = poll_result {
//             let mut attrs: Vec<KeyValue> = vec![];
//             for (k, v) in config.otel_attributes() {
//                 attrs.push(KeyValue::new(k, v));
//             }

//             for metric in r {
//                 if let Value::F64(metric_value) = metric.value {
//                     info!(
//                         "Observing metric {} with value {} and attributes {:?}",
//                         name,
//                         metric_value,
//                         attrs.as_slice()
//                     );
//                     obs.observe_f64(&guage, metric_value, &attrs);
//                 }
//             }
//         }

//         // obs.observe_f64(&guage, 1.0, &[])
//     })?;

//     // meter_store.push(meter);

//     Ok(meter)
// }
