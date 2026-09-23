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
    let body = build_request(config, now);
    let (start_time, end_time) = get_interval_times(now);

    let response = client.post("api/v1/telemetry/GroupBys", body).await?;

    info!("processing timeseries response: {}", response);

    Ok(response_to_metric_batch(
        &response, start_time, end_time, config,
    ))
}

fn build_request(config: &TSPollerConfig, now: DateTime<Utc>) -> Value {
    json!(
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
    )
}

fn response_to_metric_batch(
    response: &Value,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    config: &TSPollerConfig,
) -> IntersightMetricBatch {
    let mut ret = vec![];

    if let Value::Array(results) = response {
        if results.is_empty() {
            warn!(
                "Druid returned 0 results for interval {}/{}",
                start_time.to_rfc3339(),
                end_time.to_rfc3339()
            );
        }
        for result in results {
            info!("processing timeseries result: {}", result);
            let mut resource_metrics = IntersightResourceMetrics::default();

            if let Some(Value::Object(event)) = result.get("event") {
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
                                        v.as_str()
                                            .map(String::from)
                                            .unwrap_or_else(|| v.to_string()),
                                    )),
                                }),
                                key_strindex: 0,
                            })
                        }
                    }
                }

                for field_name in config.field_names.as_slice() {
                    let f64value = match event.get(field_name) {
                        None => {
                            warn!("Field '{}' not found in Druid event, skipping", field_name);
                            continue;
                        }
                        Some(Value::Null) => {
                            debug!(
                                "Field '{}' is null in Druid result (possible division-by-zero \
                                in post-aggregation), skipping",
                                field_name
                            );
                            continue;
                        }
                        Some(Value::Number(n)) => {
                            if let Some(v) = n.as_f64() {
                                v
                            } else if let Some(v) = n.as_i64() {
                                v as f64
                            } else {
                                warn!(
                                    "Field '{}' has unsupported numeric type, skipping",
                                    field_name
                                );
                                continue;
                            }
                        }
                        Some(other) => {
                            warn!(
                                "Field '{}' has unexpected non-numeric type in Druid result, skipping: {}",
                                field_name, other
                            );
                            continue;
                        }
                    };

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

                ret.push(resource_metrics);
            } else {
                warn!("Druid result has no 'event' object, skipping: {}", result);
            }
        }
    } else {
        warn!(
            "Druid response was not a JSON array — possible API error or unexpected format \
            (full response logged above at info level)"
        );
    }

    ret
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
    let end_aligned = (end_time - Duration::minutes(end_minutes % 10))
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap();

    // Align start to 10-minute boundary: go back 25 minutes and round down to nearest 10-minute boundary
    let start_time = now + Duration::minutes(-25);
    let start_minutes = start_time.minute() as i64;
    let start_aligned = (start_time - Duration::minutes(start_minutes % 10))
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap();

    (start_aligned, end_aligned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> TSPollerConfig {
        serde_json::from_value(json!({
            "name": "thermal",
            "datasource": "physicalstats",
            "dimensions": ["device", "slot"],
            "field_names": ["temperature", "power"],
            "filter": {"type": "selector", "dimension": "kind", "value": "server"},
            "aggregations": [{"type": "doubleSum", "name": "power", "fieldName": "watts"}],
            "post_aggregations": [{"type": "fieldAccess", "name": "temperature", "fieldName": "temp"}],
            "otel_dimension_to_attribute_map": {
                "device": "device.id",
                "slot": "hardware.slot"
            }
        }))
        .unwrap()
    }

    fn times() -> (DateTime<Utc>, DateTime<Utc>) {
        (
            Utc.with_ymd_and_hms(2025, 1, 15, 14, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2025, 1, 15, 14, 10, 0).unwrap(),
        )
    }

    fn string_attribute<'a>(resource: &'a IntersightResourceMetrics, key: &str) -> Option<&'a str> {
        resource
            .attributes
            .iter()
            .find(|attribute| attribute.key == key)
            .and_then(|attribute| attribute.value.as_ref())
            .and_then(|value| value.value.as_ref())
            .and_then(|value| match value {
                any_value::Value::StringValue(value) => Some(value.as_str()),
                _ => None,
            })
    }

    #[test]
    fn build_request_converts_config_and_interval() {
        let now = Utc.with_ymd_and_hms(2025, 1, 15, 14, 37, 45).unwrap();

        assert_eq!(
            build_request(&config(), now),
            json!({
                "queryType": "groupBy",
                "dataSource": "physicalstats",
                "dimensions": ["device", "slot"],
                "filter": {"type": "selector", "dimension": "kind", "value": "server"},
                "granularity": "all",
                "intervals": ["2025-01-15T14:10:00+00:00/2025-01-15T14:20:00+00:00"],
                "aggregations": [{"type": "doubleSum", "name": "power", "fieldName": "watts"}],
                "postAggregations": [{"type": "fieldAccess", "name": "temperature", "fieldName": "temp"}]
            })
        );
    }

    #[test]
    fn response_converts_event_metrics_dimensions_and_timestamps() {
        let config = config();
        let (start, end) = times();
        let batch = response_to_metric_batch(
            &json!([{"event": {
                "device": "server-1",
                "slot": 2,
                "temperature": 42.5,
                "power": 300
            }}]),
            start,
            end,
            &config,
        );

        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].metrics.len(), 2);
        assert_eq!(string_attribute(&batch[0], "device.id"), Some("server-1"));
        assert_eq!(string_attribute(&batch[0], "hardware.slot"), Some("2"));
        assert_eq!(batch[0].metrics[0].name, "temperature");
        assert_eq!(batch[0].metrics[0].value, 42.5);
        assert_eq!(batch[0].metrics[1].name, "power");
        assert_eq!(batch[0].metrics[1].value, 300.0);
        for metric in &batch[0].metrics {
            assert!(metric.attributes.is_empty());
            assert_eq!(metric.timestamp_offset, 15 * 60);
            assert_eq!(metric.start_time, start.into());
            assert_eq!(metric.time, end.into());
        }
    }

    #[test]
    fn response_skips_missing_null_and_non_numeric_fields() {
        let config = config();
        let (start, end) = times();

        for event in [
            json!({"power": 1}),
            json!({"temperature": null, "power": 1}),
            json!({"temperature": "hot", "power": 1}),
            json!({"temperature": true, "power": 1}),
        ] {
            let batch = response_to_metric_batch(&json!([{"event": event}]), start, end, &config);
            assert_eq!(batch.len(), 1);
            assert_eq!(batch[0].metrics.len(), 1);
            assert_eq!(batch[0].metrics[0].name, "power");
        }
    }

    #[test]
    fn response_handles_non_events_non_arrays_and_multiple_rows() {
        let config = config();
        let (start, end) = times();

        for response in [json!({"event": {}}), json!(null), json!("error")] {
            assert!(response_to_metric_batch(&response, start, end, &config).is_empty());
        }

        let batch = response_to_metric_batch(
            &json!([
                {"event": {"temperature": 10, "power": 20}},
                {"timestamp": "2025-01-15T14:00:00Z"},
                {"event": null},
                {"event": {"temperature": 30.25, "power": 40}}
            ]),
            start,
            end,
            &config,
        );
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].metrics[0].value, 10.0);
        assert_eq!(batch[1].metrics[0].value, 30.25);
    }

    #[test]
    fn empty_array_produces_empty_batch() {
        let config = config();
        let (start, end) = times();
        assert!(response_to_metric_batch(&json!([]), start, end, &config).is_empty());
    }

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

    #[test]
    fn interval_crosses_calendar_boundaries_and_remains_ten_minutes() {
        let cases = [
            (
                Utc.with_ymd_and_hms(2025, 5, 2, 0, 5, 59).unwrap(),
                "2025-05-01T23:40:00+00:00/2025-05-01T23:50:00+00:00",
            ),
            (
                Utc.with_ymd_and_hms(2025, 3, 1, 0, 5, 1).unwrap(),
                "2025-02-28T23:40:00+00:00/2025-02-28T23:50:00+00:00",
            ),
            (
                Utc.with_ymd_and_hms(2025, 1, 1, 0, 5, 0).unwrap(),
                "2024-12-31T23:40:00+00:00/2024-12-31T23:50:00+00:00",
            ),
            (
                Utc.with_ymd_and_hms(2024, 3, 1, 0, 5, 0).unwrap(),
                "2024-02-29T23:40:00+00:00/2024-02-29T23:50:00+00:00",
            ),
        ];

        for (now, expected) in cases {
            let (start, end) = get_interval_times(now);
            assert_eq!(get_interval(now), expected);
            assert_eq!(end - start, Duration::minutes(10));
            assert_eq!(start.minute() % 10, 0);
            assert_eq!(end.minute() % 10, 0);
            assert_eq!((start.second(), start.nanosecond()), (0, 0));
            assert_eq!((end.second(), end.nanosecond()), (0, 0));
        }
    }
}
