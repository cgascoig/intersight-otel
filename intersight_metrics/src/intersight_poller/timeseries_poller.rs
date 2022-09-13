use std::collections::{BTreeMap, HashMap};

use crate::config::TSPollerConfig;

use super::IntersightMetric;
use anyhow::Result;
use intersight_api::Client;
use serde_json::{json, Value};

pub async fn poll(client: &Client, config: &TSPollerConfig) -> Result<Vec<IntersightMetric>> {
    let mut aggregations: Vec<HashMap<String, String>> = vec![];

    for field_name in config.field_names.as_slice() {
        let mut aggregation: HashMap<String, String> = HashMap::new();
        aggregation.insert(String::from("type"), String::from("longLast"));
        aggregation.insert(String::from("name"), field_name.clone());
        aggregation.insert(String::from("fieldName"), field_name.clone());
        aggregations.push(aggregation);
    }
    let body = json!(  {
      "queryType": "groupBy",
      "dataSource": config.datasource,
      "dimensions": config.dimensions,
      "intervals": [ "2022-09-06T00:00:00.000/2022-09-10T00:00:00.000" ],
      "granularity": "all",
      "aggregations": aggregations,
    });

    let response = client.post("api/v1/telemetry/GroupBys", body).await?;

    // Example response
    // [
    //   {
    //     "version": "v1",
    //     "timestamp": "2022-09-06T00:00:00.000Z",
    //     "event": {
    //       "usedStorageBytes": 3895694721024,
    //       "clusterName": "CPOC-HX"
    //     }
    //   }
    // ]

    let mut ret: Vec<IntersightMetric> = vec![];
    if let Value::Array(results) = response {
        for result in results {
            if let Value::Object(event) = &result["event"] {
                // Any field of the event that isn't a value (i.e. in field_names) is an attribute/dimension of the metrics
                let mut attributes: BTreeMap<String, String> = BTreeMap::new();
                for (k, v) in event {
                    if !config.field_names.contains(k) {
                        attributes.insert(k.clone(), v.to_string());
                    }
                }

                for field_name in config.field_names.as_slice() {
                    if let Value::Number(value) = &event[field_name] {
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
