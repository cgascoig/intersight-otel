use crate::intersight_poller::IntersightMetric;
use opentelemetry::metrics::ObservableGauge;
use opentelemetry::runtime;
use opentelemetry::sdk::export::metrics::aggregation::cumulative_temporality_selector;
use opentelemetry::sdk::metrics::controllers::BasicController;
use opentelemetry::{global, Value};
use opentelemetry::{metrics, Key, KeyValue};
use opentelemetry_otlp::{ExportConfig, WithExportConfig};
use std::time::{Duration, SystemTime};
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
        let mut otel_observers = HashMap::<String, ObservableGauge<f64>>::new();
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

                    let guage = meter.f64_observable_gauge(&metric.name).init();
                    otel_observers.insert(metric_name.clone(), guage.clone());
                    meter
                        .register_callback(move |cx| {
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

                                        if metric.timestamp_offset != 0 {
                                            let cx = cx.with_value(
                                                SystemTime::now()
                                                    .checked_sub(Duration::from_secs(900))
                                                    .unwrap_or(SystemTime::now()),
                                            );

                                            guage.observe(&cx, metric_value, attrs.as_slice());
                                        } else {
                                            guage.observe(cx, metric_value, attrs.as_slice());
                                        }
                                    } else {
                                        trace!("value not f64");
                                    }
                                }
                                im.remove(&metric_name);
                            } else {
                                trace!("value not Some");
                            }
                        })
                        .unwrap();
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

fn init_metrics_otlp(otel_collector_endpoint: &str) -> metrics::Result<BasicController> {
    let export_config = ExportConfig {
        endpoint: otel_collector_endpoint.to_string(),
        ..ExportConfig::default()
    };

    opentelemetry_otlp::new_pipeline()
        .metrics(
            opentelemetry::sdk::metrics::selectors::simple::inexpensive(),
            cumulative_temporality_selector(),
            runtime::Tokio,
        )
        .with_period(Duration::from_secs(60))
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_export_config(export_config),
        )
        .build()
}
