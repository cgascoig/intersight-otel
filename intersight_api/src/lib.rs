pub mod config;
pub mod simplesigner;

use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::simplesigner::{Signer, SignerError};
use http_signature_normalization_reqwest::prelude::*;

use base64::prelude::*;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE},
    Request, Response,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Error as IoError;

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
    fn from_key_bytes(
        key_id: &str,
        pem: &[u8],
        passphrase: Option<&[u8]>,
        host: &str,
        accept_invalid_certs: bool,
    ) -> Result<Self, IntersightError> {
        let signer;
        if let Some(_passphrase) = passphrase {
            // Encrypted private key support is unimplemented
            return Err(IntersightError::KeyError);
        } else {
            signer = Signer::from_pem(pem).map_err(|e| {
                print!("SignerError: {e}");
                IntersightError::KeyError
            })?;
        }

        let signer = Arc::new(signer);

        let signing_config = Config::default()
            .require_header("host")
            .require_digest()
            .dont_use_created_field();

        let client = reqwest::Client::builder()
            .connection_verbose(true)
            .danger_accept_invalid_certs(accept_invalid_certs)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| IntersightError::ClientError)?;

        Ok(Client {
            key_id: key_id.to_string(),
            signer,
            signing_config,
            client,
            host: host.to_string(),
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

    async fn build_request(
        &self,
        method: Method,
        path: &str,
        headers: Option<&HashMap<String, String>>,
    ) -> Result<Request, IntersightError> {
        let url = format!("https://{}/{}", self.host, path.trim_start_matches('/'));

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

        let digest = format!(
            "SHA-256={}",
            BASE64_STANDARD.encode(Sha256::digest(body_str.as_bytes()))
        );

        let mut request_headers = HeaderMap::new();
        if let Some(headers) = headers {
            for (k, v) in headers {
                let hv = HeaderValue::from_str(v).map_err(|_| {
                    IntersightError::InvalidParamater("Invalid header value".to_string())
                })?;
                let hn = k.to_lowercase().parse::<HeaderName>().map_err(|_| {
                    IntersightError::InvalidParamater("Invalid header name".to_string())
                })?;
                request_headers.insert(hn, hv);
            }
        }

        if !request_headers.contains_key("date") {
            request_headers.insert(
                "date",
                HeaderValue::from_str(
                    &httpdate::HttpDate::from(std::time::SystemTime::now()).to_string(),
                )
                .map_err(|_| {
                    IntersightError::InvalidParamater("Invalid header value".to_string())
                })?,
            );
        }

        if !request_headers.contains_key("user-agent") {
            request_headers.insert("user-agent", HeaderValue::from_static("intersight-otel"));
        }

        if !request_headers.contains_key("accept") {
            request_headers.insert("accept", HeaderValue::from_static("application/json"));
        }

        if !body_str.is_empty() && !request_headers.contains_key(CONTENT_TYPE) {
            request_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }

        let signer = self.signer.clone();

        req.headers(request_headers)
            .header("Host", &self.host)
            .header("Digest", digest)
            .authorization_signature(&self.signing_config, &self.key_id, move |s| {
                trace!(
                    "String to sign for authorization header: \n-------\n{}\n-------",
                    s
                );
                let b64 = BASE64_STANDARD.encode(
                    signer
                        .sign_to_vec(s.as_bytes())
                        .map_err(IntersightError::Sign)?,
                );
                trace!("Calculated signature: {}", b64);
                Ok(b64) as Result<_, IntersightError>
            })
            .await
    }

    async fn call(&self, method: Method, path: &str) -> Result<Value, IntersightError> {
        let req = self.build_request(method, path, None).await?;

        trace!("Request built: {:#?}", req);

        let response = self.client.execute(req).await?;

        Self::parse_response(response).await
    }

    async fn parse_response(response: Response) -> Result<Value, IntersightError> {
        if !response.status().is_success() {
            let status = response.status().as_u16();
            if let Ok(body) = response.text().await {
                trace!("Intersight API error response body: {}", body);
            }
            return Err(IntersightError::ApiError(status));
        }

        let body = response.bytes().await.map_err(IntersightError::Body)?;
        serde_json::from_slice(&body).map_err(IntersightError::ResponseError)
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
    #[error("Invalid parameter for Intersight API")]
    InvalidParamater(String),

    #[error("error reading private key")]
    KeyReadError(#[from] IoError),

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

    #[error("Intersight API returned HTTP {0}")]
    ApiError(u16),
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::{Method as HttpMethod, StatusCode};
    use serde_json::json;

    const TEST_KEY: &[u8] = include_bytes!("../tests/examples/example-v2.pem");
    const EMPTY_DIGEST: &str = "SHA-256=47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=";
    const DATE: &str = "Thu, 23 Jun 2022 00:57:07 GMT";

    fn client() -> Client {
        Client::from_key_bytes("key-id", TEST_KEY, None, "intersight.example", false)
            .expect("client should build")
    }

    fn headers() -> HashMap<String, String> {
        HashMap::from([("date".to_string(), DATE.to_string())])
    }

    async fn request(method: Method) -> Request {
        client()
            .build_request(method, "/api/v1/items", Some(&headers()))
            .await
            .expect("request should build")
    }

    #[tokio::test]
    async fn get_builds_signed_request_with_defaults() {
        let request = request(Method::Get).await;

        assert_eq!(request.method(), HttpMethod::GET);
        assert_eq!(
            request.url().as_str(),
            "https://intersight.example/api/v1/items"
        );
        assert!(request.body().is_none());
        assert_eq!(request.headers()["host"], "intersight.example");
        assert_eq!(request.headers()["date"], DATE);
        assert_eq!(request.headers()["accept"], "application/json");
        assert_eq!(request.headers()["user-agent"], "intersight-otel");
        assert_eq!(request.headers()["digest"], EMPTY_DIGEST);
        assert!(request.headers().get(CONTENT_TYPE).is_none());
        let authorization = request.headers()["authorization"]
            .to_str()
            .expect("authorization should be text");
        assert!(authorization.starts_with("Signature keyId=\"key-id\",algorithm=\"hs2019\""));
        assert!(authorization.contains("(request-target) accept date digest host user-agent"));
    }

    #[tokio::test]
    async fn body_methods_serialize_json_and_set_content_type() {
        let body = json!({"enabled": true, "count": 2});
        let expected_body = serde_json::to_vec(&body).expect("JSON should serialize");
        let expected_digest = format!(
            "SHA-256={}",
            BASE64_STANDARD.encode(Sha256::digest(&expected_body))
        );

        for (method, expected_method) in [
            (Method::Post(body.clone()), HttpMethod::POST),
            (Method::Patch(body.clone()), HttpMethod::PATCH),
        ] {
            let request = request(method).await;
            assert_eq!(request.method(), expected_method);
            assert_eq!(request.headers()[CONTENT_TYPE], "application/json");
            assert_eq!(request.headers()["digest"], expected_digest);
            assert_eq!(
                request.body().and_then(reqwest::Body::as_bytes),
                Some(expected_body.as_slice())
            );
        }
    }

    #[tokio::test]
    async fn delete_has_no_body_and_custom_headers_override_defaults() {
        let mut custom_headers = headers();
        custom_headers.insert("Accept".to_string(), "application/problem+json".to_string());
        custom_headers.insert("User-Agent".to_string(), "test-agent".to_string());
        let request = client()
            .build_request(Method::Delete, "api/v1/items/1", Some(&custom_headers))
            .await
            .expect("request should build");

        assert_eq!(request.method(), HttpMethod::DELETE);
        assert_eq!(request.headers()["accept"], "application/problem+json");
        assert_eq!(request.headers()["user-agent"], "test-agent");
        assert_eq!(request.headers()["digest"], EMPTY_DIGEST);
        assert!(request.body().is_none());
    }

    #[tokio::test]
    async fn rejects_invalid_header_name_and_value() {
        let invalid_name = HashMap::from([("bad header".to_string(), "value".to_string())]);
        let error = client()
            .build_request(Method::Get, "path", Some(&invalid_name))
            .await
            .expect_err("invalid header names should fail");
        assert!(
            matches!(error, IntersightError::InvalidParamater(message) if message == "Invalid header name")
        );

        let invalid_value = HashMap::from([("x-test".to_string(), "bad\nvalue".to_string())]);
        let error = client()
            .build_request(Method::Get, "path", Some(&invalid_value))
            .await
            .expect_err("invalid header values should fail");
        assert!(
            matches!(error, IntersightError::InvalidParamater(message) if message == "Invalid header value")
        );
    }

    fn response(status: StatusCode, body: &'static str) -> Response {
        reqwest::Response::from(
            http::Response::builder()
                .status(status)
                .body(body)
                .expect("test response should build"),
        )
    }

    #[tokio::test]
    async fn parses_successful_json_response() {
        let value = Client::parse_response(response(StatusCode::OK, r#"{"value":42}"#))
            .await
            .expect("valid JSON response should parse");
        assert_eq!(value, json!({"value": 42}));
    }

    #[tokio::test]
    async fn reports_http_error_status_before_parsing_body() {
        let error = Client::parse_response(response(StatusCode::BAD_GATEWAY, "not JSON"))
            .await
            .expect_err("error status should fail");
        assert!(matches!(error, IntersightError::ApiError(502)));
    }

    #[tokio::test]
    async fn reports_malformed_success_response() {
        let error = Client::parse_response(response(StatusCode::OK, "not JSON"))
            .await
            .expect_err("malformed JSON should fail");
        assert!(matches!(error, IntersightError::ResponseError(_)));
    }
}
