use clap::Parser;
use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct ResultMappingConfig {
    pub result_field: String,
    pub result_attribute: String,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct AttributeEnricherConfig {
    pub name: String,
    pub source_attribute: String,
    pub source_value_regex: Option<String>,
    pub query_template: String,
    pub result_mappings: Vec<ResultMappingConfig>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct GlobalConfig {
    pub key_file: String,
    key_id: String,
    pub intersight_host: Option<String>,
    pub intersight_accept_invalid_certs: Option<bool>,
    pub otel_collector_endpoint: String,
    pub pollers: Option<Vec<PollerConfig>>,
    pub tspollers: Option<Vec<TSPollerConfig>>,
    pub enrichers: Option<Vec<AttributeEnricherConfig>>,
}

impl GlobalConfig {
    pub fn new() -> Result<Self, ConfigError> {
        let args = Args::parse();

        let c = Config::builder()
            .add_source(File::with_name(&args.config_file))
            .add_source(Environment::with_prefix("intersight_otel"))
            .build()?;

        c.try_deserialize()
    }

    pub fn key_id(&self) -> &str {
        self.key_id.trim()
    }
}

pub type OTelAttributes = HashMap<String, String>;

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct PollerConfig {
    pub api_query: String,
    pub api_method: Option<String>,
    pub api_body: Option<String>,
    pub aggregator: String,
    pub aggregator_options: Option<HashMap<String, String>>,
    pub name: String,
    pub otel_attributes: Option<HashMap<String, String>>,
    pub enrichers: Option<Vec<String>>,

    interval: Option<u64>, // interval is private with a getter because it might change to human strings like "5m" in the future
}

impl PollerConfig {
    pub fn interval(&self) -> u64 {
        self.interval.unwrap_or(10).max(1)
    }
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long, value_parser, default_value_t = String::from("intersight_otel"))]
    config_file: String,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct TSPollerConfig {
    pub name: String,
    pub datasource: String,
    pub dimensions: Vec<String>,
    pub field_names: Vec<String>,
    pub filter: Option<Value>,
    pub aggregations: Option<Value>,
    pub post_aggregations: Option<Value>,
    poller_type: Option<String>,
    interval: Option<u64>,

    pub otel_attributes: Option<HashMap<String, String>>,
    pub otel_dimension_to_attribute_map: Option<HashMap<String, String>>,
    pub enrichers: Option<Vec<String>>,
}

pub enum TSPollerType {
    LastValue,
    Sum,
}

impl TSPollerConfig {
    #[allow(unused)]
    pub fn poller_type(&self) -> TSPollerType {
        if let Some(t) = &self.poller_type {
            return match t.as_str() {
                "sum" => TSPollerType::Sum,
                _ => TSPollerType::LastValue,
            };
        }

        TSPollerType::LastValue
    }

    pub fn interval(&self) -> u64 {
        self.interval.unwrap_or(10).max(1)
    }
}

pub trait OTelAttributeProvider {
    fn otel_attributes(&self) -> OTelAttributes;
}

impl OTelAttributeProvider for PollerConfig {
    fn otel_attributes(&self) -> OTelAttributes {
        self.otel_attributes.clone().unwrap_or_default()
    }
}

impl OTelAttributeProvider for TSPollerConfig {
    fn otel_attributes(&self) -> OTelAttributes {
        self.otel_attributes.clone().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::FileFormat;
    use serde_json::json;

    fn poller(value: Value) -> PollerConfig {
        serde_json::from_value(value).unwrap()
    }

    fn tspoller(value: Value) -> TSPollerConfig {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn poller_config_deserializes_optional_fields_and_attributes() {
        let config = poller(json!({
            "api_query": "api/v1/example",
            "api_method": "post",
            "api_body": "{\"Enabled\":true}",
            "aggregator": "result_count",
            "aggregator_options": { "field": "Count" },
            "name": "example.count",
            "otel_attributes": { "site": "lab" },
            "enrichers": ["server"],
            "interval": 30
        }));

        assert_eq!(config.api_query, "api/v1/example");
        assert_eq!(config.api_method.as_deref(), Some("post"));
        assert_eq!(config.api_body.as_deref(), Some("{\"Enabled\":true}"));
        assert_eq!(config.interval(), 30);
        assert_eq!(
            config.otel_attributes().get("site").map(String::as_str),
            Some("lab")
        );
        assert_eq!(
            config.enrichers.as_deref(),
            Some(["server".to_string()].as_slice())
        );
    }

    #[test]
    fn poller_config_defaults_interval_and_attributes() {
        let config = poller(json!({
            "api_query": "api/v1/example",
            "aggregator": "count_results",
            "name": "example.count"
        }));

        assert_eq!(config.interval(), 10);
        assert!(config.otel_attributes().is_empty());
    }

    #[test]
    fn zero_intervals_are_clamped_to_one_second() {
        let poller = poller(json!({
            "api_query": "api/v1/example",
            "aggregator": "result_count",
            "name": "example.count",
            "interval": 0
        }));
        let tspoller = tspoller(json!({
            "name": "example.value",
            "datasource": "example",
            "dimensions": [],
            "field_names": [],
            "interval": 0
        }));

        assert_eq!(poller.interval(), 1);
        assert_eq!(tspoller.interval(), 1);
    }

    #[test]
    fn global_config_trims_key_id() {
        let config: GlobalConfig = serde_json::from_value(json!({
            "key_file": "key.pem",
            "key_id": "  key-id/1\n",
            "otel_collector_endpoint": "http://localhost:4317"
        }))
        .unwrap();

        assert_eq!(config.key_id(), "key-id/1");
        assert!(config.pollers.is_none());
        assert!(config.tspollers.is_none());
        assert!(config.enrichers.is_none());
    }

    #[test]
    fn example_config_deserializes() {
        let source = concat!(
            "key_file = \"test-key.pem\"\nkey_id = \"test-key\"\n",
            include_str!("../../examples/intersight_otel.toml")
        );
        let config: GlobalConfig = Config::builder()
            .add_source(File::from_str(source, FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert!(!config.pollers.unwrap().is_empty());
        assert!(!config.tspollers.unwrap().is_empty());
    }
}
