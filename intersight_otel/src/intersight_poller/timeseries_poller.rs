use std::collections::BTreeMap;

use crate::{config::TSPollerConfig, intersight_poller::IntersightResourceMetrics};

use super::{IntersightMetric, IntersightMetricBatch};
use anyhow::Result;
use chrono::{prelude::*, Duration};
use intersight_api::Client;
use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, KeyValue, KeyValueList};
use serde_json::{json, Value};

pub async fn poll(client: &Client, config: &TSPollerConfig) -> Result<IntersightMetricBatch> {
    let body = json!(
        {
            "queryType": "groupBy",
            "dataSource": config.datasource,
            "dimensions": config.dimensions,
            "filter": config.filter,
            "granularity": {"type":"period","period":"PT10M","timeZone":"America/Los_Angeles"},
            "intervals": [ get_interval() ],
            "aggregations": config.aggregations,
            "postAggregations": config.post_aggregations,
        }
    );

    let response = client.post("api/v1/telemetry/GroupBys", body).await?;

    info!("processing timeseries response: {}", response);

    let mut ret: IntersightMetricBatch = vec![];

    if let Value::Array(results) = response {
        for result in results {
            info!("processing timeseries result: {}", result);
            let mut resource_metrics = IntersightResourceMetrics::default();

            if let Value::Object(event) = &result["event"] {
                // Apply dimension to attribute mapping
                let mut attributes: BTreeMap<String, String> = BTreeMap::new();
                if let Some(otel_dimension_to_attribute_map) =
                    &config.otel_dimension_to_attribute_map
                {
                    for (dimension_name, attribute_name) in otel_dimension_to_attribute_map {
                        if let Some(v) = event.get(dimension_name) {
                            // attributes.insert(attribute_name.clone(), v.to_string());
                            resource_metrics.attributes.push(KeyValue {
                                key: attribute_name.clone(),
                                value: Some(AnyValue {
                                    value: Some(any_value::Value::StringValue(v.to_string())),
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

                        let mut metric =
                            IntersightMetric::new(field_name, f64value, Some(attributes.clone()));

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

fn get_interval() -> String {
    // The interval is always the 10 minute interval that started 25 minutes ago.
    // This is to ensure that all the data complete in the Druid results.
    let end = (Utc::now() + Duration::minutes(-15)).to_rfc3339();
    let start = (Utc::now() + Duration::minutes(-25)).to_rfc3339();
    format!("{start}/{end}")
}
