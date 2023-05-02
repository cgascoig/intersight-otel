use crate::intersight_poller::IntersightMetric;
use opentelemetry::sdk::metrics::PushController;
use opentelemetry::{global, metrics::ValueObserver, Value};
use opentelemetry::{metrics, Key, KeyValue};
use opentelemetry_otlp::{ExportConfig, WithExportConfig};
use std::time::Duration;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

pub fn start_metric_merger(
    mut metric_chan: Receiver<IntersightMetric>,
    otel_collector_endpoint: &str,
) -> JoinHandle<()> {
    let otel_collector_endpoint = String::from(otel_collector_endpoint);
    tokio::spawn(async move {
        let _ctrl = init_metrics_otlp(&otel_collector_endpoint);

        let intersight_metrics: HashMap<String, Vec<IntersightMetric>> = HashMap::new();
        let intersight_metrics = Arc::new(Mutex::new(intersight_metrics));
        let mut otel_observers = HashMap::<String, ValueObserver<f64>>::new();
        let meter = global::meter("intersight");

        info!("Starting metric merger task");
        loop {
            if let Some(metric) = metric_chan.recv().await {
                info!("Received metric {} = {}", metric.name, metric.value);

                let metric_name = metric.name.clone();

                // if we haven't seen this metric before, create an otel observer for it
                if !otel_observers.contains_key(&metric_name) {
                    debug!("Adding new otel observer for metric {}", metric_name);

                    let intersight_metrics_ref = intersight_metrics.clone();

                    otel_observers.insert(
                        metric_name.clone(),
                        meter
                            .f64_value_observer(metric.name.clone(), move |r| {
                                trace!("i64_value_observer called");
                                let mut im = intersight_metrics_ref.lock().unwrap();
                                let metrics = im.get(&metric_name);
                                if let Some(metrics) = metrics {
                                    for metric in metrics {
                                        if let Value::F64(metric_value) = metric.value {
                                            trace!("value observed");

                                            let mut attrs: Vec<KeyValue> = vec![];
                                            for (key, value) in metric.attributes.clone() {
                                                attrs.push(KeyValue {
                                                    key: Key::from(key),
                                                    value: Value::from(value),
                                                })
                                            }

                                            trace!(
                                                "observing value {} with attributes {:?}",
                                                metric_value,
                                                attrs.as_slice()
                                            );
                                            r.observe(metric_value, attrs.as_slice());
                                        } else {
                                            trace!("value not f64");
                                        }
                                    }
                                    im.remove(&metric_name);
                                } else {
                                    trace!("value not Some");
                                }
                            })
                            .init(),
                    );
                }

                // update the intersight_metrics hashmap with the new value
                let mut im = intersight_metrics.lock().unwrap();
                if let Some(metrics) = im.get_mut(&metric.name) {
                    metrics.push(metric);
                } else {
                    im.insert(metric.name.clone(), vec![metric]);
                }
            }
        }
    })
}

fn init_metrics_otlp(otel_collector_endpoint: &str) -> metrics::Result<PushController> {
    let export_config = ExportConfig {
        endpoint: otel_collector_endpoint.to_string(),
        ..ExportConfig::default()
    };
    opentelemetry_otlp::new_pipeline()
        .metrics(tokio::spawn, opentelemetry::util::tokio_interval_stream)
        .with_period(Duration::from_secs(60))
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_export_config(export_config),
        )
        .build()
}

// use futures_util::{Stream, StreamExt as _};
// use std::time::Duration;
// use opentelemetry::sdk::export;

// // Skip first immediate tick from tokio, not needed for async_std.
// fn delayed_interval(duration: Duration) -> impl Stream<Item = tokio::time::Instant> {
//     opentelemetry::util::tokio_interval_stream(duration).skip(1)
// }

// fn init_metrics_stdout() -> metrics::Result<PushController> {
//     let exporter = export::metrics::stdout(tokio::spawn, delayed_interval)
//         .with_period(Duration::from_secs(5))
//         .init();

//     Ok(exporter)
// }
