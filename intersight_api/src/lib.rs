pub mod simplesigner;

use std::sync::Arc;

use crate::simplesigner::{Signer, SignerError};
use http_signature_normalization_reqwest::prelude::*;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[macro_use]
extern crate log;

#[derive(Clone)]
pub struct Client {
    signer: Arc<Signer>,
    key_id: String,
    signing_config: Config,
    client: reqwest::Client,
    host: String,
}

impl Client {
    pub fn from_key_bytes(
        key_id: &str,
        pem: &[u8],
        passphrase: Option<&[u8]>,
    ) -> Result<Self, IntersightError> {
        let signer;
        if let Some(_passphrase) = passphrase {
            // Encrypted private key support is unimplemented
            return Err(IntersightError::KeyError);
        } else {
            signer = Signer::from_pem(pem).map_err(|_| IntersightError::KeyError)?;
        }

        let signer = Arc::new(signer);

        let signing_config = Config::default()
            .require_header("host")
            .require_digest()
            .dont_use_created_field();

        let client = reqwest::Client::builder()
            .connection_verbose(true)
            .build()
            .map_err(|_| IntersightError::ClientError)?;

        Ok(Client {
            key_id: key_id.to_string(),
            signer,
            signing_config,
            client,
            host: "intersight.com".to_string(),
        })
    }

    pub async fn get(&self, path: &str) -> Result<Value, IntersightError> {
        self.call(Method::Get, path).await
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<Value, IntersightError> {
        self.call(Method::Post(body), path).await
    }

    pub async fn patch(&self, path: &str, body: Value) -> Result<Value, IntersightError> {
        self.call(Method::Patch(body), path).await
    }

    pub async fn delete(&self, path: &str) -> Result<Value, IntersightError> {
        self.call(Method::Delete, path).await
    }

    async fn call(&self, method: Method, path: &str) -> Result<Value, IntersightError> {
        let url = format!("https://{}/{}", self.host, path);

        let mut body: Option<serde_json::Value> = None;

        let mut req: reqwest::RequestBuilder = match method {
            Method::Get => self.client.get(url),
            Method::Post(b) => {
                body = Some(b);
                self.client.post(url)
            }
            Method::Patch(b) => {
                body = Some(b);
                self.client.patch(url)
            }
            Method::Delete => self.client.delete(url),
        };

        let mut body_str = "".to_string();
        if let Some(body) = body {
            body_str = serde_json::to_string(&body)?;
            req = req.body(body_str.clone())
        }

        let mut hasher = Sha256::new();
        let digest = format!("SHA-256={}", hasher.compute(body_str.as_bytes()));

        let req = req
            .header("Host", &self.host)
            .header(
                "Date",
                httpdate::HttpDate::from(std::time::SystemTime::now()).to_string(),
            )
            .header("Digest", digest)
            .authorization_signature(&self.signing_config, &self.key_id, |s| {
                trace!(
                    "String to sign for authorization header: \n-------\n{}\n-------",
                    s
                );

                let b64 = base64::encode(
                    self.signer
                        .sign_to_vec(s.as_bytes())
                        .map_err(IntersightError::Sign)?,
                );
                trace!("Calculated signature: {}", b64);
                Ok(b64) as Result<_, IntersightError>
            })?
            .header("User-Agent", "Reqwest")
            .header("Accept", "application/json");

        trace!("Request built: {:#?}", req);

        let response = req.send().await?;

        let body = response.bytes().await.map_err(IntersightError::Body)?;

        let js = serde_json::from_slice(&body).map_err(IntersightError::ResponseError)?;

        Ok(js)
    }
}

enum Method {
    Get,
    Post(serde_json::Value),
    Patch(serde_json::Value),
    Delete,
}

#[derive(thiserror::Error, Debug)]
pub enum IntersightError {
    #[error("error loading private key")]
    KeyError,

    #[error("error setting up API client")]
    ClientError,

    #[error("Failed to create signing string, {0}")]
    Convert(#[from] SignError),

    #[error("Failed to send request: {0}")]
    SendRequest(#[from] reqwest::Error),

    #[error("Failed to retrieve request body")]
    Body(reqwest::Error),

    #[error("Failed to sign string")]
    Sign(SignerError),

    #[error("Failed to parse response: {0}")]
    ResponseError(#[from] serde_json::Error),
}
