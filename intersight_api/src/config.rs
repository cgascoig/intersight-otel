use crate::{Client, IntersightError};
use std::fs;

#[derive(Default)]
pub struct Config {
    key_id: Option<String>,
    key_data: Option<Vec<u8>>,
    host: Option<String>,
    insecure: Option<bool>,
}

impl Config {
    pub fn new() -> Self {
        Config::default()
    }

    pub fn with_key_id(self, key_id: &str) -> Self {
        Config {
            key_id: Some(key_id.to_string()),
            ..self
        }
    }

    pub fn with_key_bytes(self, key_bytes: &[u8]) -> Self {
        Config {
            key_data: Some(key_bytes.to_vec()),
            ..self
        }
    }

    pub fn with_key_file(self, key_file_name: &str) -> Result<Self, IntersightError> {
        let key_bytes = fs::read(key_file_name)?;
        Ok(Config {
            key_data: Some(key_bytes),
            ..self
        })
    }

    pub fn with_insecure(self, insecure: bool) -> Self {
        Config {
            insecure: Some(insecure),
            ..self
        }
    }

    pub fn with_host(self, host: &str) -> Self {
        Config {
            host: Some(host.to_string()),
            ..self
        }
    }

    pub fn build_client(self) -> Result<Client, IntersightError> {
        let host = self.host.unwrap_or("intersight.com".to_string());
        let key_id = self
            .key_id
            .ok_or_else(|| IntersightError::InvalidParamater("Key ID is required".to_string()))?;
        let pem = self.key_data.ok_or_else(|| IntersightError::KeyError)?;
        let passphrase = None;
        let accept_invalid_certs = self.insecure.unwrap_or(true);

        Client::from_key_bytes(
            &key_id,
            pem.as_ref(),
            passphrase,
            host.as_ref(),
            accept_invalid_certs,
        )
    }
}

#[test]
fn test_config() {
    let key_id = "1234/1234/1234";
    let config = Config::new().with_key_id(key_id);
    assert_eq!(key_id, config.key_id.as_ref().unwrap());

    let key_bytes: &[u8] = "12345".as_bytes();
    let config = config.with_key_bytes(key_bytes);
    assert_eq!(key_id, config.key_id.as_ref().unwrap());
    assert_eq!(key_bytes, config.key_data.as_ref().unwrap());

    let key_file_name = "tests/examples/example-v2.pem";
    let config = config
        .with_key_file(key_file_name)
        .expect("test key file not found");
    assert_eq!(key_id, config.key_id.as_ref().unwrap());
    let test_key_bytes = fs::read(key_file_name).expect("test key file not found");
    assert_eq!(&test_key_bytes, config.key_data.as_ref().unwrap());

    let config = config.with_host("intersight.local").with_insecure(true);
    assert!(config.insecure.unwrap());
    assert_eq!("intersight.local", config.host.unwrap());
}
