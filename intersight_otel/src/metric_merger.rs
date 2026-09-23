use crate::intersight_poller::{IntersightMetricBatch, IntersightResourceMetrics};

use opentelemetry_proto::tonic::common::v1::any_value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use opentelemetry_proto::tonic::metrics::v1::number_data_point::Value;
use opentelemetry_proto::{
    self,
    tonic::{
        collector::metrics::v1::{
            metrics_service_client::MetricsServiceClient, ExportMetricsServiceRequest,
        },
        common::v1::InstrumentationScope,
        metrics::v1::{
            metric::Data, Gauge, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics,
        },
        resource::v1::Resource,
    },
};
use std::{future::Future, time::SystemTime};
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;
use tonic::transport::{self, Channel};

pub fn start_metric_merger(
    metric_chan: Receiver<IntersightMetricBatch>,
    otel_collector_endpoint: &str,
) -> JoinHandle<()> {
    let otel_collector_endpoint = String::from(otel_collector_endpoint);
    tokio::spawn(async move {
        let client = match init_metrics_client(&otel_collector_endpoint).await {
            Ok(client) => client,
            Err(err) => {
                error!("Failed to initialise metrics client: {}", err);
                return;
            }
        };

        info!("Starting metric merger task");
        run_metric_merger_loop(metric_chan, move |resource_metrics| {
            let mut client = client.clone();
            async move {
                let res = client
                    .export(ExportMetricsServiceRequest {
                        resource_metrics: vec![resource_metrics],
                    })
                    .await;

                if let Err(err) = res {
                    error!("Error sending metrics: {}", err);
                }
            }
        })
        .await;
    })
}

async fn run_metric_merger_loop<F, Fut>(
    mut metric_chan: Receiver<IntersightMetricBatch>,
    mut export: F,
) where
    F: FnMut(ResourceMetrics) -> Fut,
    Fut: Future<Output = ()>,
{
    while let Some(metric_batch) = metric_chan.recv().await {
        for rm in metric_batch {
            info!(
                "Received resource metrics {:?} = {:?}",
                rm.attributes, rm.metrics
            );
            export(ResourceMetrics::from(rm)).await;
        }
    }
}

async fn init_metrics_client(
    otel_collector_endpoint: &str,
) -> Result<MetricsServiceClient<Channel>, tonic::transport::Error> {
    let endpoint = transport::channel::Endpoint::from_shared(otel_collector_endpoint.to_string())?;
    let channel = endpoint.connect().await?;
    // let mut client = MetricsServiceClient::connect(channel).await?;
    Ok(MetricsServiceClient::new(channel))
}

impl From<IntersightResourceMetrics> for ResourceMetrics {
    fn from(value: IntersightResourceMetrics) -> Self {
        let mut metrics = vec![];
        // let start_time = value.start_time.unwrap_or(SystemTime::now());
        for m in value.metrics {
            let attributes = m
                .attributes
                .into_iter()
                .map(|(key, value)| KeyValue {
                    key,
                    value: Some(AnyValue {
                        value: Some(any_value::Value::StringValue(value)),
                    }),
                    key_strindex: 0,
                })
                .collect();
            metrics.push(Metric {
                name: m.name,
                description: "".to_string(),
                unit: "".to_string(),
                metadata: vec![],
                data: Some(Data::Gauge(Gauge {
                    data_points: vec![NumberDataPoint {
                        attributes,
                        start_time_unix_nano: m
                            .start_time
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_nanos() as u64,
                        time_unix_nano: m
                            .time
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_nanos() as u64,
                        // start_time_unix_nano: start_time
                        //     .checked_sub(Duration::from_secs(m.timestamp_offset))
                        //     .expect("Unable to apply timestamp offset")
                        //     .duration_since(UNIX_EPOCH)
                        //     .expect("Time went backwards")
                        //     .as_nanos() as u64,
                        // time_unix_nano: SystemTime::now()
                        //     .checked_sub(Duration::from_secs(m.timestamp_offset))
                        //     .expect("Unable to apply timestamp offset")
                        //     .duration_since(UNIX_EPOCH)
                        //     .expect("Time went backwards")
                        //     .as_nanos() as u64,
                        exemplars: vec![],
                        flags: 0,
                        value: Some(Value::AsDouble(m.value)),
                    }],
                })),
            })
        }
        let mut resource_attributes = value.attributes;
        resource_attributes.push(KeyValue {
            key: "telemetry.sdk.name".to_string(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue("intersight-otel".to_string())),
            }),
            key_strindex: 0,
        });
        ResourceMetrics {
            resource: Some(Resource {
                attributes: resource_attributes,
                ..Default::default()
            }),
            schema_url: "".to_string(),
            scope_metrics: vec![ScopeMetrics {
                scope: Some(InstrumentationScope {
                    name: "".to_string(),
                    version: "".to_string(),
                    attributes: vec![],
                    dropped_attributes_count: 0,
                }),
                metrics,
                schema_url: "".to_string(),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intersight_poller::IntersightMetric;
    use std::{collections::BTreeMap, sync::Arc, time::Duration};
    use tokio::{sync::mpsc, time::timeout};

    fn string_attribute(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(value.to_string())),
            }),
            key_strindex: 0,
        }
    }

    fn gauge_data_point(metric: &Metric) -> &NumberDataPoint {
        match metric.data.as_ref() {
            Some(Data::Gauge(gauge)) => &gauge.data_points[0],
            _ => panic!("expected gauge metric"),
        }
    }

    #[test]
    fn converts_resource_and_metric_data_to_otlp() {
        let start_time = SystemTime::UNIX_EPOCH + Duration::from_nanos(1_234);
        let time = SystemTime::UNIX_EPOCH + Duration::from_nanos(5_678);
        let metric = IntersightMetric::new(
            "intersight.power.watts",
            42.5,
            Some(BTreeMap::from([
                ("device.id".to_string(), "abc".to_string()),
                ("state".to_string(), "active".to_string()),
            ])),
            start_time,
            time,
        );
        let resource_metrics = ResourceMetrics::from(IntersightResourceMetrics {
            attributes: vec![string_attribute("service.name", "intersight")],
            metrics: vec![metric],
            start_time: None,
        });

        let resource = resource_metrics.resource.expect("resource must be present");
        assert_eq!(
            resource.attributes,
            vec![
                string_attribute("service.name", "intersight"),
                string_attribute("telemetry.sdk.name", "intersight-otel"),
            ]
        );
        assert_eq!(resource_metrics.scope_metrics.len(), 1);
        let metrics = &resource_metrics.scope_metrics[0].metrics;
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].name, "intersight.power.watts");

        let point = gauge_data_point(&metrics[0]);
        assert_eq!(point.value, Some(Value::AsDouble(42.5)));
        assert_eq!(point.start_time_unix_nano, 1_234);
        assert_eq!(point.time_unix_nano, 5_678);
        assert_eq!(
            point.attributes,
            vec![
                string_attribute("device.id", "abc"),
                string_attribute("state", "active"),
            ]
        );
    }

    #[test]
    fn converts_each_metric_independently() {
        let resource_metrics = ResourceMetrics::from(IntersightResourceMetrics {
            attributes: vec![],
            metrics: vec![
                IntersightMetric::new(
                    "first",
                    -1.25,
                    None,
                    SystemTime::UNIX_EPOCH,
                    SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                ),
                IntersightMetric::new(
                    "second",
                    f64::INFINITY,
                    Some(BTreeMap::from([("kind".to_string(), "peak".to_string())])),
                    SystemTime::UNIX_EPOCH + Duration::from_secs(2),
                    SystemTime::UNIX_EPOCH + Duration::from_secs(3),
                ),
            ],
            start_time: None,
        });

        let metrics = &resource_metrics.scope_metrics[0].metrics;
        assert_eq!(
            metrics.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert_eq!(
            gauge_data_point(&metrics[0]).value,
            Some(Value::AsDouble(-1.25))
        );
        assert!(gauge_data_point(&metrics[0]).attributes.is_empty());
        assert_eq!(
            gauge_data_point(&metrics[1]).value,
            Some(Value::AsDouble(f64::INFINITY))
        );
        assert_eq!(
            gauge_data_point(&metrics[1]).start_time_unix_nano,
            2_000_000_000
        );
        assert_eq!(gauge_data_point(&metrics[1]).time_unix_nano, 3_000_000_000);
        assert_eq!(
            gauge_data_point(&metrics[1]).attributes,
            vec![string_attribute("kind", "peak")]
        );
    }

    #[test]
    fn converts_empty_metrics_without_dropping_resource_attributes() {
        let resource_metrics = ResourceMetrics::from(IntersightResourceMetrics {
            attributes: vec![string_attribute("host.name", "server-1")],
            metrics: vec![],
            start_time: None,
        });

        assert!(resource_metrics.scope_metrics[0].metrics.is_empty());
        assert_eq!(
            resource_metrics.resource.unwrap().attributes,
            vec![
                string_attribute("host.name", "server-1"),
                string_attribute("telemetry.sdk.name", "intersight-otel"),
            ]
        );
    }

    #[tokio::test]
    async fn receiver_loop_exports_batches_and_exits_when_channel_closes() {
        let (sender, receiver) = mpsc::channel(1);
        sender
            .send(vec![IntersightResourceMetrics {
                attributes: vec![],
                metrics: vec![],
                start_time: None,
            }])
            .await
            .unwrap();
        drop(sender);

        let exported = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = Arc::clone(&exported);
        timeout(
            Duration::from_secs(1),
            run_metric_merger_loop(receiver, move |metrics| {
                let captured = Arc::clone(&captured);
                async move {
                    captured.lock().unwrap().push(metrics);
                }
            }),
        )
        .await
        .expect("receiver loop did not exit after channel closure");

        assert_eq!(exported.lock().unwrap().len(), 1);
    }
}
