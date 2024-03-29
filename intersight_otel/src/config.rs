use clap::Parser;
use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

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
        return self.key_id.trim();
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
        match self.interval {
            Some(x) => x,
            _ => 10,
        }
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
