use crate::intersight_poller::IntersightMetric;
use opentelemetry::{global, metrics::ValueObserver, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

pub fn merge_metrics(mut metric_chan: Receiver<IntersightMetric>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let intersight_metrics: HashMap<String, Value> = HashMap::new();
        let intersight_metrics = Arc::new(Mutex::new(intersight_metrics));
        let mut otel_observers = HashMap::<String, ValueObserver<i64>>::new();
        let meter = global::meter("intersight");

        info!("Starting metric merger task");
        loop {
            if let Some(metric) = metric_chan.recv().await {
                info!("Received metric {} = {}", metric.name, metric.value);

                // if we haven't seen this metric before, create an otel observer for it
                if !otel_observers.contains_key(&metric.name) {
                    debug!("Adding new otel observer for metric {}", metric.name);
                    let intersight_metrics_ref = intersight_metrics.clone();
                    let metric_name = metric.name.clone();
                    otel_observers.insert(
                        metric.name.clone(),
                        meter
                            .i64_value_observer(metric_name.clone(), move |r| {
                                let im = intersight_metrics_ref.lock().unwrap();
                                let metric = im.get(&metric_name);
                                if let Some(metric) = metric {
                                    if let Value::I64(metric) = metric {
                                        r.observe(*metric, &[]);
                                    }
                                }
                            })
                            .init(),
                    );
                }

                // update the intersight_metrics hashmap with the new value
                let mut im = intersight_metrics.lock().unwrap();
                im.insert(metric.name, metric.value);
            }
        }
    })
}
