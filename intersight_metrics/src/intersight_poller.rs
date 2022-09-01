use generic_poller::{ResultCountAggregator, ResultCountingAggregator};
use intersight_api::Client;
use opentelemetry::{global, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::time;

mod generic_poller;

#[derive(Debug)]
pub struct IntersightMetric {
    name: String,
    value: Value,
}

pub async fn start_poller(client: &Client) {
    let intersight_metrics: HashMap<String, Value> = HashMap::new();
    let intersight_metrics = Arc::new(Mutex::new(intersight_metrics));
    let mut otel_observers = HashMap::new();

    //We run the pollers once first to get the metric names
    let new_metrics = run_polls(client).await;

    let meter = global::meter("intersight");

    //Create an otel observer for each metric
    for metric in new_metrics {
        let intersight_metrics_ref = intersight_metrics.clone();

        let metric_name = metric.name.clone();
        otel_observers.insert(
            metric.name.clone(),
            meter
                .i64_value_observer(metric.name.clone(), move |r| {
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

        let mut intersight_metrics = intersight_metrics.lock().unwrap();
        intersight_metrics.insert(metric.name.clone(), metric.value);
    }

    let mut interval = time::interval(time::Duration::from_secs(10));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        interval.tick().await;

        let new_metrics = run_polls(client).await;
        let mut intersight_metrics = intersight_metrics.lock().unwrap();
        intersight_metrics.clear();
        for metric in new_metrics {
            intersight_metrics.insert(metric.name, metric.value);
        }
    }
}

async fn run_polls(client: &Client) -> Vec<IntersightMetric> {
    info!("Running Intersight pollers");

    let mut ret: Vec<IntersightMetric> = Vec::new();

    let poll_result = generic_poller::poll(
        client,
        "api/v1/ntp/Policies",
        &ResultCountingAggregator::new("ntp_policy_count".to_string()),
    )
    .await;

    if let Ok(r) = poll_result {
        ret.extend(r);
    } else if let Err(err) = poll_result {
        error!("error while polling Intersight: {}", err);
    }

    let poll_result = generic_poller::poll(
        client,
        "api/v1/virtualization/VirtualMachines?$count=true",
        &ResultCountAggregator::new("virtual_machine_count".to_string()),
    )
    .await;

    if let Ok(r) = poll_result {
        ret.extend(r);
    } else if let Err(err) = poll_result {
        error!("error while polling Intersight: {}", err);
    }

    info!("Finished running Intersight pollers");

    return ret;
}
