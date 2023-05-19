use super::IntersightMetric;
use intersight_api::{Client, IntersightError};
use serde_json::Value;

pub async fn poll(
    client: &Client,
    query: &str,
    method: &Option<String>,
    body: &Option<String>,
    agg: &(dyn Aggregator + Sync + Send),
) -> Result<Vec<IntersightMetric>, PollerError> {
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
    fn aggregate(&self, r: Value) -> Vec<IntersightMetric>;
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
    fn aggregate(&self, r: Value) -> Vec<IntersightMetric> {
        let count: i64;
        if let serde_json::Value::Array(results) = &r["Results"] {
            count = results.len() as i64;
        } else {
            return Vec::new();
        }

        let m = IntersightMetric::new(&self.name, opentelemetry::Value::F64(count as f64), None);

        vec![m]
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

        let m = IntersightMetric::new(&self.name, opentelemetry::Value::F64(count as f64), None);

        vec![m]
    }
}
