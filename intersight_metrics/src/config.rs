use config::{Config, ConfigError, File};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct GlobalConfig {
    pub key_file: String,
    pub key_id: String,
    pub pollers: Option<Vec<PollerConfig>>,
}

impl GlobalConfig {
    pub fn new() -> Result<Self, ConfigError> {
        let c = Config::builder()
            .add_source(File::with_name("ismetrics"))
            .build()?;

        c.try_deserialize()
    }
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct PollerConfig {
    pub api_query: String,
    pub aggregator: String,
    pub aggregator_options: Option<HashMap<String, String>>,
    pub name: String,

    interval: Option<u64>, // interval is private with a getter because it might change to human strings like "5m" in the future
}

impl PollerConfig {
    pub fn interval(&self) -> u64 {
        match self.interval {
            Some(x) => x,
            _ => 10,
        }
    }
}
