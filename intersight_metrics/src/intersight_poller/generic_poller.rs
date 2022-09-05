use super::IntersightMetric;
use intersight_api::{Client, IntersightError};
use serde_json::Value;

pub async fn poll(
    client: &Client,
    query: &str,
    agg: &(dyn Aggregator + Sync + Send),
) -> Result<Vec<IntersightMetric>, PollerError> {
    let response = client.get(query).await.map_err(PollerError::APIError)?;

    let ret = agg.aggregate(response);

    return Ok(ret);
}

#[derive(thiserror::Error, Debug)]
pub enum PollerError {
    #[error("error calling Intersight API")]
    APIError(IntersightError),
}

pub trait Aggregator {
    fn aggregate(&self, r: Value) -> Vec<IntersightMetric>;
}

//ResultCountingAggregator will explicitly count the number of results returned
pub struct ResultCountingAggregator {
    name: String,
}

impl ResultCountingAggregator {
    pub fn new(name: String) -> ResultCountingAggregator {
        return ResultCountingAggregator { name: name };
    }
}

impl Aggregator for ResultCountingAggregator {
    fn aggregate(&self, r: Value) -> Vec<IntersightMetric> {
        let count: i64;
        if let serde_json::Value::Array(results) = &r["Results"] {
            count = results.len() as i64;
        } else {
            return Vec::new();
        }

        let m = IntersightMetric {
            name: self.name.clone(),
            value: opentelemetry::Value::I64(count),
        };

        vec![m]
    }
}

//ResultCountAggregator will extract the "Count" field from the returned data
pub struct ResultCountAggregator {
    name: String,
}

impl ResultCountAggregator {
    pub fn new(name: String) -> ResultCountAggregator {
        return ResultCountAggregator { name: name };
    }
}

impl Aggregator for ResultCountAggregator {
    fn aggregate(&self, r: Value) -> Vec<IntersightMetric> {
        let count: i64;
        if let serde_json::Value::Number(c) = &r["Count"] {
            if let Some(c) = c.as_i64() {
                count = c;
            } else {
                warn!("Unexpected type for result count");
                return Vec::new();
            }
        } else {
            warn!("'Count' field not present in API response. Did you mean to include '$count=true' in the API query?");
            return Vec::new();
        }

        let m = IntersightMetric {
            name: self.name.clone(),
            value: opentelemetry::Value::I64(count),
        };

        vec![m]
    }
}
