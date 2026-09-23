use super::{IntersightMetric, IntersightMetricBatch, IntersightResourceMetrics};
use intersight_api::{Client, IntersightError};
use serde_json::Value;
use std::time::SystemTime;

pub async fn poll(
    client: &Client,
    query: &str,
    method: &Option<String>,
    body: &Option<String>,
    agg: &(dyn Aggregator + Sync + Send),
) -> Result<IntersightMetricBatch, PollerError> {
    let method = (*method).clone().unwrap_or_default();
    let method = method.as_str();

    let body = match body {
        Some(b) => b.as_str(),
        _ => "",
    };
    let body = match body {
        "" => Value::Null,
        _ => serde_json::from_str(body).map_err(|_| PollerError::ConfigError)?,
    };

    let response = match method {
        "post" => client
            .post(query, body)
            .await
            .map_err(PollerError::APIError)?,
        _ => client.get(query).await.map_err(PollerError::APIError)?,
    };

    let ret = agg.aggregate(response);

    Ok(ret)
}

#[derive(thiserror::Error, Debug)]
pub enum PollerError {
    #[error("error calling Intersight API")]
    APIError(IntersightError),

    #[error("poller configuration error")]
    ConfigError,
}

pub trait Aggregator {
    fn aggregate(&self, r: Value) -> IntersightMetricBatch;
}

//ResultCountingAggregator will explicitly count the number of results returned
pub struct ResultCountingAggregator {
    name: String,
}

impl ResultCountingAggregator {
    pub fn new(name: String) -> ResultCountingAggregator {
        ResultCountingAggregator { name }
    }
}

impl Aggregator for ResultCountingAggregator {
    fn aggregate(&self, r: Value) -> IntersightMetricBatch {
        let mut ret = IntersightResourceMetrics::default();

        let count: i64;
        if let serde_json::Value::Array(results) = &r["Results"] {
            count = results.len() as i64;
        } else {
            return vec![];
        }

        ret.metrics.push(IntersightMetric::new(
            &self.name,
            count as f64,
            None,
            SystemTime::now(),
            SystemTime::now(),
        ));

        vec![ret]
    }
}

//ResultCountAggregator will extract the "Count" field from the returned data
pub struct ResultCountAggregator {
    name: String,
}

impl ResultCountAggregator {
    pub fn new(name: String) -> ResultCountAggregator {
        ResultCountAggregator { name }
    }
}

impl Aggregator for ResultCountAggregator {
    fn aggregate(&self, r: Value) -> IntersightMetricBatch {
        let mut ret = IntersightResourceMetrics::default();

        let count: i64;
        if let serde_json::Value::Number(c) = &r["Count"] {
            if let Some(c) = c.as_i64() {
                count = c;
            } else {
                warn!("Unexpected type for result count");
                return vec![];
            }
        } else {
            warn!("'Count' field not present in API response. Did you mean to include '$count=true' in the API query?");
            return vec![];
        }

        ret.metrics.push(IntersightMetric::new(
            &self.name,
            count as f64,
            None,
            SystemTime::now(),
            SystemTime::now(),
        ));

        vec![ret]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assert_single_metric(batch: IntersightMetricBatch, name: &str, value: f64) {
        assert_eq!(batch.len(), 1);
        assert!(batch[0].attributes.is_empty());
        assert!(batch[0].start_time.is_none());
        assert_eq!(batch[0].metrics.len(), 1);

        let metric = &batch[0].metrics[0];
        assert_eq!(metric.name, name);
        assert_eq!(metric.value, value);
        assert!(metric.attributes.is_empty());
    }

    #[test]
    fn result_count_aggregator_extracts_integer_count() {
        let aggregator = ResultCountAggregator::new("objects.total".to_string());

        assert_single_metric(
            aggregator.aggregate(json!({ "Count": 42 })),
            "objects.total",
            42.0,
        );
    }

    #[test]
    fn result_count_aggregator_rejects_missing_or_invalid_count() {
        let aggregator = ResultCountAggregator::new("objects.total".to_string());

        assert!(aggregator.aggregate(json!({})).is_empty());
        assert!(aggregator.aggregate(json!({ "Count": "42" })).is_empty());
        assert!(aggregator.aggregate(json!({ "Count": 1.5 })).is_empty());
        assert!(aggregator
            .aggregate(json!({ "Count": 9_223_372_036_854_775_808_u64 }))
            .is_empty());
    }

    #[test]
    fn result_counting_aggregator_counts_results() {
        let aggregator = ResultCountingAggregator::new("objects.returned".to_string());

        assert_single_metric(
            aggregator.aggregate(json!({ "Results": [{}, {}, {}] })),
            "objects.returned",
            3.0,
        );
        assert_single_metric(
            aggregator.aggregate(json!({ "Results": [] })),
            "objects.returned",
            0.0,
        );
    }

    #[test]
    fn result_counting_aggregator_rejects_missing_or_non_array_results() {
        let aggregator = ResultCountingAggregator::new("objects.returned".to_string());

        assert!(aggregator.aggregate(json!({})).is_empty());
        assert!(aggregator
            .aggregate(json!({ "Results": { "Count": 2 } }))
            .is_empty());
    }
}
