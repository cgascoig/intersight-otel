use super::{IntersightMetric, IntersightMetricBatch, IntersightResourceMetrics};
use intersight_api::{Client, IntersightError};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::task;

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

pub fn poll_sync(
    client: &Client,
    query: &str,
    method: &Option<String>,
    body: &Option<String>,
    agg: &(dyn Aggregator + Sync + Send),
) -> Result<IntersightMetricBatch, PollerError> {
    task::block_in_place(move || {
        Handle::current().block_on(async move { poll(client, query, method, body, agg).await })
    })
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

        ret.metrics
            .push(IntersightMetric::new(&self.name, count as f64, None));

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

        ret.metrics
            .push(IntersightMetric::new(&self.name, count as f64, None));

        vec![ret]
    }
}
