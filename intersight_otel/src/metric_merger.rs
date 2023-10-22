use crate::intersight_poller::{IntersightMetricBatch, IntersightResourceMetrics};

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
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;
use tonic::transport::{self, Channel};

pub fn start_metric_merger(
    mut metric_chan: Receiver<IntersightMetricBatch>,
    otel_collector_endpoint: &str,
) -> JoinHandle<()> {
    let otel_collector_endpoint = String::from(otel_collector_endpoint);
    tokio::spawn(async move {
        let _ctrl = init_metrics_client(&otel_collector_endpoint).await;
        let mut client;
        if let Err(err) = _ctrl {
            error!("Failed to initialise metrics client: {}", err);
            return;
        } else {
            client = _ctrl.unwrap();
        }

        info!("Starting metric merger task");
        loop {
            if let Some(metric_batch) = metric_chan.recv().await {
                for rm in metric_batch {
                    info!(
                        "Received resouce metrics {:?} = {:?}",
                        rm.attributes, rm.metrics
                    );

                    let resource_metrics = ResourceMetrics::from(rm);

                    let res = client
                        .export(ExportMetricsServiceRequest {
                            resource_metrics: vec![resource_metrics],
                        })
                        .await;

                    if let Err(err) = res {
                        error!("Error sending metrics: {}", err);
                    }
                }
            }
        }
    })
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
        let start_time = value.start_time.unwrap_or(SystemTime::now());
        for m in value.metrics {
            metrics.push(Metric {
                name: m.name,
                description: "".to_string(),
                unit: "".to_string(),
                data: Some(Data::Gauge(Gauge {
                    data_points: vec![NumberDataPoint {
                        attributes: vec![],
                        start_time_unix_nano: start_time
                            .checked_sub(Duration::from_secs(m.timestamp_offset))
                            .expect("Unable to apply timestamp offset")
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_nanos() as u64,
                        time_unix_nano: SystemTime::now()
                            .checked_sub(Duration::from_secs(m.timestamp_offset))
                            .expect("Unable to apply timestamp offset")
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_nanos() as u64,
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
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                        "intersight-otel".to_string(),
                    ),
                ),
            }),
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
