use std::collections::BTreeMap;

use crate::config::TSPollerConfig;

use super::IntersightMetric;
use anyhow::Result;
use chrono::{prelude::*, Duration};
use intersight_api::Client;
use serde_json::{json, Value};

pub async fn poll(client: &Client, config: &TSPollerConfig) -> Result<Vec<IntersightMetric>> {
    let body = json!(
        {
            "queryType": "groupBy",
            "dataSource": "hx",
            "dimensions": ["deviceId"],
            "filter": config.filter,
            "granularity": {"type":"period","period":"PT5M","timeZone":"America/Los_Angeles"},
            "intervals": [ get_interval() ],
            "aggregations": config.aggregations,
            "postAggregations": config.post_aggregations,
        }
    );

    let response = client.post("api/v1/telemetry/GroupBys", body).await?;

    info!("processing timeseries response: {}", response);

    let mut ret: Vec<IntersightMetric> = vec![];
    if let Value::Array(results) = response {
        for result in results {
            info!("processing timeseries result: {}", result);

            if let Value::Object(event) = &result["event"] {
                // Apply dimension to attribute mapping
                let mut attributes: BTreeMap<String, String> = BTreeMap::new();
                if let Some(otel_dimension_to_attribute_map) =
                    &config.otel_dimension_to_attribute_map
                {
                    for (dimension_name, attribute_name) in otel_dimension_to_attribute_map {
                        if let Some(v) = event.get(dimension_name) {
                            attributes.insert(attribute_name.clone(), v.to_string());
                        }
                    }
                }

                for field_name in config.field_names.as_slice() {
                    if let Some(Value::Number(value)) = event.get(field_name) {
                        let otel_value;
                        if let Some(value) = value.as_f64() {
                            otel_value = opentelemetry::Value::F64(value);
                        } else if let Some(value) = value.as_i64() {
                            otel_value = opentelemetry::Value::I64(value);
                        } else {
                            continue;
                        }

                        let metric =
                            IntersightMetric::new(field_name, otel_value, Some(attributes.clone()));

                        ret.push(metric);
                    }
                }
            }
        }
    }

    Ok(ret)
}

fn get_interval() -> String {
    // The interval is always the 5 minute interval that started 20 minutes ago.
    // This is to ensure that all the data complete in the Druid results.
    let end = (Utc::now() + Duration::minutes(-15)).to_rfc3339();
    let start = (Utc::now() + Duration::minutes(-20)).to_rfc3339();
    format!("{start}/{end}")
}
