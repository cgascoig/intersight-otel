use crate::intersight_poller::{
    IntersightMetric, IntersightMetricBatch, IntersightResourceMetrics,
};

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
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use std::{
    error::Error,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
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
        let mut resource_attributes = value.attributes.clone();
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

// Just a test ... should remove this
impl From<IntersightMetric> for ResourceMetrics {
    fn from(value: IntersightMetric) -> Self {
        ResourceMetrics {
            resource: Some(Resource {
                attributes: vec![KeyValue {
                    key: "k1".to_string(),
                    value: Some(AnyValue {
                        value: Some(
                            opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                                "v1".to_string(),
                            ),
                        ),
                    }),
                }],
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
                metrics: vec![Metric {
                    name: value.name,
                    description: "".to_string(),
                    unit: "".to_string(),
                    data: Some(Data::Gauge(Gauge {
                        data_points: vec![NumberDataPoint {
                            attributes: vec![],
                            start_time_unix_nano: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .expect("Time went backwards")
                                .as_nanos()
                                as u64,
                            time_unix_nano: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .expect("Time went backwards")
                                .as_nanos() as u64,
                            exemplars: vec![],
                            flags: 0,
                            value: Some(Value::AsDouble(value.value)),
                        }],
                    })),
                }],
                schema_url: "".to_string(),
            }],
        }
    }
}

// pub fn init_metrics_otlp(otel_collector_endpoint: &str) -> metrics::Result<MeterProvider> {
//     let export_config = ExportConfig {
//         endpoint: otel_collector_endpoint.to_string(),
//         ..ExportConfig::default()
//     };

//     opentelemetry_otlp::new_pipeline()
//         .metrics(runtime::Tokio)
//         .with_period(Duration::from_secs(10))
//         .with_exporter(
//             opentelemetry_otlp::new_exporter()
//                 .tonic()
//                 .with_export_config(export_config),
//         )
//         // .with_aggregation_selector(selectors::simple::Selector::Exact)
//         .build()
// }

// pub fn start_metric_merger(
//     mut metric_chan: Receiver<IntersightMetric>,
//     otel_collector_endpoint: &str,
// ) -> Option<JoinHandle<()>> {
//     let otel_collector_endpoint = String::from(otel_collector_endpoint);
//     tokio::spawn(async move {
//         let _ctrl = init_metrics_client(&otel_collector_endpoint).await;
//         if let Err(_err) = _ctrl {
//             return None;
//         }

//         // let intersight_metrics: HashMap<String, Vec<IntersightMetric>> = HashMap::new();
//         // let intersight_metrics = Arc::new(Mutex::new(intersight_metrics));
//         // let mut otel_observers = HashMap::<String, ObservableGauge<f64>>::new();
//         // let meter = global::meter("");

//         info!("Starting metric merger task");
//         loop {
//             if let Some(metric) = metric_chan.recv().await {
//                 info!("Received metric {} = {}", metric.name, metric.value);

//                 // let metric_name = metric.name.clone();

//                 // // if we haven't seen this metric before, create an otel observer for it
//                 // if !otel_observers.contains_key(&metric_name) {
//                 //     debug!("Adding new otel observer for metric {}", metric_name);

//                 //     let intersight_metrics_ref = intersight_metrics.clone();

//                 //     let guage = meter.f64_observable_gauge(&metric.name).init();
//                 //     otel_observers.insert(metric_name.clone(), guage.clone());
//                 //     meter
//                 //         .register_callback(move |cx| {
//                 //             trace!("i64_value_observer called");
//                 //             let mut im = intersight_metrics_ref.lock().unwrap();
//                 //             let metrics = im.get(&metric_name);
//                 //             if let Some(metrics) = metrics {
//                 //                 for metric in metrics {
//                 //                     if let Value::F64(metric_value) = metric.value {
//                 //                         trace!("value observed");

//                 //                         let mut attrs: Vec<KeyValue> = vec![];
//                 //                         for (key, value) in metric.attributes.clone() {
//                 //                             attrs.push(KeyValue {
//                 //                                 key: Key::from(key),
//                 //                                 value: Value::from(value),
//                 //                             })
//                 //                         }

//                 //                         trace!(
//                 //                             "observing value {} with attributes {:?}",
//                 //                             metric_value,
//                 //                             attrs.as_slice()
//                 //                         );

//                 //                         if metric.timestamp_offset != 0 {
//                 //                             let cx = cx.with_value(
//                 //                                 SystemTime::now()
//                 //                                     .checked_sub(Duration::from_secs(900))
//                 //                                     .unwrap_or(SystemTime::now()),
//                 //                             );

//                 //                             guage.observe(&cx, metric_value, attrs.as_slice());
//                 //                         } else {
//                 //                             guage.observe(cx, metric_value, attrs.as_slice());
//                 //                         }
//                 //                     } else {
//                 //                         trace!("value not f64");
//                 //                     }
//                 //                 }
//                 //                 im.remove(&metric_name);
//                 //             } else {
//                 //                 trace!("value not Some");
//                 //             }
//                 //         })
//                 //         .unwrap();
//                 // }

//                 // // update the intersight_metrics hashmap with the new value
//                 // let mut im = intersight_metrics.lock().unwrap();
//                 // if let Some(metrics) = im.get_mut(&metric.name) {
//                 //     metrics.push(metric);
//                 // } else {
//                 //     im.insert(metric.name.clone(), vec![metric]);
//                 // }
//             }
//         }
//     })
// }
