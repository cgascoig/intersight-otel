use clap::Parser;
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
        let args = Args::parse();

        let c = Config::builder()
            .add_source(File::with_name(&args.config_file))
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

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long, value_parser, default_value_t = String::from("ismetrics"))]
    config_file: String,
}
