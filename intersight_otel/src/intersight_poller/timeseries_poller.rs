use std::collections::BTreeMap;

use crate::{config::TSPollerConfig, intersight_poller::IntersightResourceMetrics};

use super::{IntersightMetric, IntersightMetricBatch};
use anyhow::Result;
use chrono::{prelude::*, Duration};
use intersight_api::Client;
use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, KeyValue};
use serde_json::{json, Value};

pub async fn poll(client: &Client, config: &TSPollerConfig) -> Result<IntersightMetricBatch> {
    let now = Utc::now();
    let body = json!(
        {
            "queryType": "groupBy",
            "dataSource": config.datasource,
            "dimensions": config.dimensions,
            "filter": config.filter,
            "granularity": "all",
            "intervals": [ get_interval(now) ],
            "aggregations": config.aggregations,
            "postAggregations": config.post_aggregations,
        }
    );
    let (start_time, end_time) = get_interval_times(now);

    let response = client.post("api/v1/telemetry/GroupBys", body).await?;

    info!("processing timeseries response: {}", response);

    let mut ret: IntersightMetricBatch = vec![];

    if let Value::Array(results) = response {
        for result in results {
            info!("processing timeseries result: {}", result);
            let mut resource_metrics = IntersightResourceMetrics::default();

            if let Value::Object(event) = &result["event"] {
                // Apply dimension to attribute mapping
                let attributes: BTreeMap<String, String> = BTreeMap::new();
                if let Some(otel_dimension_to_attribute_map) =
                    &config.otel_dimension_to_attribute_map
                {
                    for (dimension_name, attribute_name) in otel_dimension_to_attribute_map {
                        if let Some(v) = event.get(dimension_name) {
                            // attributes.insert(attribute_name.clone(), v.to_string());
                            resource_metrics.attributes.push(KeyValue {
                                key: attribute_name.clone(),
                                value: Some(AnyValue {
                                    value: Some(any_value::Value::StringValue(
                                        v.as_str().map(String::from).unwrap_or_else(|| v.to_string()),
                                    )),
                                }),
                            })
                        }
                    }
                }

                for field_name in config.field_names.as_slice() {
                    if let Some(Value::Number(value)) = event.get(field_name) {
                        let f64value: f64;
                        if let Some(value) = value.as_f64() {
                            f64value = value;
                        } else if let Some(value) = value.as_i64() {
                            f64value = value as f64;
                        } else {
                            continue;
                        }

                        let mut metric = IntersightMetric::new(
                            field_name,
                            f64value,
                            Some(attributes.clone()),
                            start_time.into(),
                            end_time.into(),
                        );

                        metric.timestamp_offset = 15 * 60;

                        resource_metrics.metrics.push(metric);
                    }
                }

                ret.push(resource_metrics);
            }
        }
    }

    Ok(ret)
}

fn get_interval(now: DateTime<Utc>) -> String {
    let (start_time, end_time) = get_interval_times(now);

    let end = end_time
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap()
        .to_rfc3339();
    let start = start_time
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap()
        .to_rfc3339();

    format!("{start}/{end}")
}

fn get_interval_times(now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    // The interval is always the 10 minute interval that started 25 minutes ago.
    // This is to ensure that all the data complete in the Druid results.
    // Start and end times are aligned to 10-minute boundaries (e.g., 01:00, 01:10, 01:20).

    // Align end to 10-minute boundary: go back 15 minutes and round down to nearest 10-minute boundary
    let end_time = now + Duration::minutes(-15);
    let end_minutes = end_time.minute() as i64;
    let end_aligned = end_time - Duration::minutes(end_minutes % 10);

    // Align start to 10-minute boundary: go back 25 minutes and round down to nearest 10-minute boundary
    let start_time = now + Duration::minutes(-25);
    let start_minutes = start_time.minute() as i64;
    let start_aligned = start_time - Duration::minutes(start_minutes % 10);

    (start_aligned, end_aligned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_interval_aligns_to_10_minute_boundaries() {
        // Test with a time at 14:37:45 UTC
        let now = Utc.with_ymd_and_hms(2025, 1, 15, 14, 37, 45).unwrap();
        let interval = get_interval(now);

        // At 14:37, end should be 15 min ago -> 14:22, aligned to 14:20
        // At 14:37, start should be 25 min ago -> 14:12, aligned to 14:10
        assert!(
            interval.eq("2025-01-15T14:10:00+00:00/2025-01-15T14:20:00+00:00"),
            "Interval: {}",
            interval
        );
    }

    #[test]
    fn test_get_interval_on_boundary() {
        // Test when current time is exactly on a boundary (14:30:00)
        let now = Utc.with_ymd_and_hms(2025, 1, 15, 14, 25, 0).unwrap();
        let interval = get_interval(now);

        // At 14:25, end should be 15 min ago -> 14:10, aligned to 14:10
        // At 14:25, start should be 25 min ago -> 14:00, aligned to 14:00
        assert!(
            interval.eq("2025-01-15T14:00:00+00:00/2025-01-15T14:10:00+00:00"),
            "Interval: {}",
            interval
        );
    }

    #[test]
    fn test_get_interval_just_over_boundary() {
        // Test when current time is exactly on a boundary (14:30:00)
        let now = Utc.with_ymd_and_hms(2025, 1, 15, 14, 25, 1).unwrap();
        let interval = get_interval(now);

        // At 14:25:01, end should be 15 min ago -> 14:10:01, aligned to 14:10
        // At 14:25:01, start should be 25 min ago -> 14:00:01, aligned to 14:00
        assert!(
            interval.eq("2025-01-15T14:00:00+00:00/2025-01-15T14:10:00+00:00"),
            "Interval: {}",
            interval
        );
    }

    #[test]
    fn test_get_interval_just_under_boundary() {
        // Test when current time is exactly on a boundary (14:30:00)
        let now = Utc.with_ymd_and_hms(2025, 1, 15, 14, 24, 59).unwrap();
        let interval = get_interval(now);

        // At 14:24:59, end should be 15 min ago -> 14:09:59, aligned to 14:00
        // At 14:24:59, start should be 25 min ago -> 13:59:59, aligned to 13:50
        assert!(
            interval.eq("2025-01-15T13:50:00+00:00/2025-01-15T14:00:00+00:00"),
            "Interval: {}",
            interval
        );
    }
}
