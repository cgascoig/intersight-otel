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
        let host = self.host.unwrap_or_else(|| "intersight.com".to_string());
        let key_id = self
            .key_id
            .ok_or_else(|| IntersightError::InvalidParamater("Key ID is required".to_string()))?;
        let pem = self.key_data.ok_or_else(|| IntersightError::KeyError)?;
        let passphrase = None;
        let accept_invalid_certs = self.insecure.unwrap_or(false);

        Client::from_key_bytes(
            &key_id,
            pem.as_ref(),
            passphrase,
            host.as_ref(),
            accept_invalid_certs,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &[u8] = include_bytes!("../tests/examples/example-v2.pem");

    fn error<T>(result: Result<T, IntersightError>, message: &str) -> IntersightError {
        match result {
            Ok(_) => panic!("{message}"),
            Err(error) => error,
        }
    }

    #[test]
    fn builder_records_values_and_reads_key_file() {
        let key_id = "1234/1234/1234";
        let key_file_name = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/examples/example-v2.pem");
        let config = Config::new()
            .with_key_id(key_id)
            .with_key_bytes(b"replaced")
            .with_key_file(key_file_name)
            .expect("test key should be readable")
            .with_host("intersight.local")
            .with_insecure(true);

        assert_eq!(config.key_id.as_deref(), Some(key_id));
        assert_eq!(config.key_data.as_deref(), Some(TEST_KEY));
        assert_eq!(config.host.as_deref(), Some("intersight.local"));
        assert_eq!(config.insecure, Some(true));
    }

    #[test]
    fn defaults_use_intersight_host_and_valid_tls_certificates() {
        let config = Config::new();
        assert!(config.host.is_none());
        assert!(!config.insecure.unwrap_or(false));

        let client = Config::new()
            .with_key_id("key-id")
            .with_key_bytes(TEST_KEY)
            .build_client()
            .expect("default configuration should build");
        assert_eq!(client.host, "intersight.com");
    }

    #[test]
    fn build_requires_key_id() {
        let error = error(
            Config::new().with_key_bytes(TEST_KEY).build_client(),
            "a key ID is required",
        );
        assert!(
            matches!(error, IntersightError::InvalidParamater(message) if message == "Key ID is required")
        );
    }

    #[test]
    fn build_requires_key_data() {
        let error = error(
            Config::new().with_key_id("key-id").build_client(),
            "key data is required",
        );
        assert!(matches!(error, IntersightError::KeyError));
    }

    #[test]
    fn key_file_read_errors_are_preserved() {
        let error = error(
            Config::new().with_key_file("a-file-that-does-not-exist.pem"),
            "missing files should fail",
        );
        assert!(matches!(error, IntersightError::KeyReadError(_)));
    }

    #[test]
    fn invalid_key_data_is_rejected() {
        let error = error(
            Config::new()
                .with_key_id("key-id")
                .with_key_bytes(b"not a PEM key")
                .build_client(),
            "invalid key data should fail",
        );
        assert!(matches!(error, IntersightError::KeyError));
    }
}
