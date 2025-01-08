pub mod config;
pub mod simplesigner;

use std::{collections::HashMap, sync::Arc};

use crate::simplesigner::{Signer, SignerError};
use http_signature_normalization_reqwest::prelude::*;

use base64::prelude::*;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Request,
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

        let response = self.client.execute(req).await?; //req.send().await?;

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
}

#[tokio::test]
async fn test_build_request_v2() {
    let v2_key_id = "59c84e4a16267c0001c23428/59cc595416267c0001a0dfc7/62b3ba347564612d3198f5b1";
    //NOSONAR
    let v2_secret_key = r#"
-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEAvV0n1s8QcR7S7u5rR94//VoUSIxJ7jvLdZRNYRQcQCECxp+H
V6ut+61D5t7YQqNcTIEv71ssC9UNs/wCIFELeN5MweLqvYto03SFJB0bLZ+ycpnp
e9jTqALZqa6uCLycFjtV9s7sW5nZZcuDiyLlNCygtkzXkUdBQ3ycaZpJphKwezQ1
xXgmWaUV6JqihSwVgj9U7sZOQN/6eCbbL2/kLoHnAYVIlbiuV0uTZsFGLsm2ZP1o
A3h2NdqhPHrBlWSmUAdhYIGlu7WNQ0yN5d6PpwHERCUI2+fOKxau8C42EYDttYf1
tnU4VZC7ItmE8ZDlrGn9f5F8virhhlBEESTXkwIDAQABAoIBABfQiVwYembfi4OE
9HT7XGzOUVK2Ye3WE0ZcOkcFMnBWNnUoRusdqinGpo14ZRYsWUU90ft2KdnrF2gV
P2c1Cg5PVrPjh8YCrFI7iyr5hht8xAJpnNV4dVXh1eHjF/v9TFv3Zl49s7fpZ0/I
AmkTIGQpYKTMkSeyIGEOYNVfE/gQljcRz6yf60GmWJY5IglXh/00GtB3GQHJqWLs
rWMi7uwtFCp6dpQDjC7VAanAnmkti4/+hiNC8c+29Zf5LcQYPz2oY3V1UlpynyYH
b+mRL5iFJwcKZs+93waTyD/igFzK+ly9Nw3/vM/D0h5wxw8UPMFHyBKN3MAI4tzW
M1QtbYECgYEA5c3V1mReeOIDx6ilUebKUooryhg0EcKIYA5bUFvlYkB7E688CpdL
nCHoeRjCKcQ0jZzpZcBpB+CoaHNLCvpaKSHzvXGmFUztMX7FMVGERWk8RwvxCVl3
j9LstVvcXklt6OE2E3GLQUhLFbs0xWghlNZWMf/KCx7t/WUChRgGRWECgYEA0vMx
EDlLISZTheR2hKlENn2yAxYfo8XieArPcjt1kivGVVqnItUMtzCRHnF1cjYnbk8g
Tf5x+8LwlOHTCX9VrQQYM98t0WsWVSmkrzss1/K0yu09sYsdOet9UL7Jet7kpA3L
dfRxXQHySJaUPVYFR9f8hQsuJrUdndFiHdHlzXMCgYEAzFNzIXgGo9bZ44mozKS3
GiKugrd4fJ4KIdZCDLZYwz5v8HWrngMeAEoJ6LpB0V8aFxwATi+Bc7amJpD0lWM6
DT6Z+MR3FpNahtqfvJUtVYYXSVhtzZFWBHRXcX2m99K0Pg8YxLr9RWNhF4Znimpn
CW52H2i+nZq3oslQL0TINqECgYA33LTScgmmNqsJmu2TxetNbs3UKWipiv6lAV/c
BUjmM3drJP17qOWcIV1crXkHjLW2bXfFj6sJm57wHjkvm6vJjHsISYKtoWkhlkyJ
JueCLECaOGcM/CT6MJVX654ZTqtHkmudyeS3V4uck1ugPoZZdyXk6YgIMhAsucT8
1pe/ZwKBgGoZAhOaR/s5EM/bwIpqPE870VnWeIbvDc8vMH3tW/q7SysfyNxyZ99w
pQ8EfDaxnEFVuY7Xa8i/qr7mmXo5E+d0TrxkB1bqtwaJJ8ojaW5G/PIkU3aTC6uV
11QYh2F1qu2ow8Y4Q3DZ78jc9M3gHvzuknyencU2K0+VhVgwEVtI
-----END RSA PRIVATE KEY-----
"#;
    let content_type = "application/json";
    let req_date = "Thu, 23 Jun 2022 00:57:07 GMT";

    let client = Client::from_key_bytes(
        v2_key_id,
        v2_secret_key.as_bytes(),
        None,
        "intersight.com",
        false,
    )
    .expect("client should build successfully");

    let mut headers = HashMap::new();
    headers.insert("date".to_string(), req_date.to_string());
    headers.insert("content-type".to_string(), content_type.to_string());
    let req = client
        .build_request(Method::Get, "/api/v1/ntp/Policies", Some(&headers))
        .await
        .expect("request should build successfully");

    assert_eq!(
        req.headers()
            .get("digest")
            .expect("digest header should be present"),
        "SHA-256=47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
    );

    assert_eq!(
        req.headers()
            .get("authorization")
            .expect("authorization header should be present"),
            "Signature keyId=\"59c84e4a16267c0001c23428/59cc595416267c0001a0dfc7/62b3ba347564612d3198f5b1\",algorithm=\"hs2019\",headers=\"(request-target) accept content-type date digest host user-agent\",signature=\"U/ixkbh+gQZ/hiTzS8WthcEYbun42AAndX5Kuq3C4omRM2+iEYWTamDL03+DDEwCovOioFJszlh8r1xxKlBwKABWTuY9fzwi9HM2s2wlm11tOo326O5gHbJRc3MWcnyICuGgH4YjK0VaNmwonSsuPydxKiOJkc1aXmQ5jKkPSDpJeGsleRT52DRGRjkb2DtKUkPRVhVNStNKYzPi7NDvGEj/B0Tq2s++8uh9vmT3RSI1DmNOR+jd9RgjZckb1cvCawUCAb6dG3N+aXgCqioG6Tm6ocE6uYqKtBNTAELom7ydlS0wGCu8NFad4shrfUW87tBfo0gE9hRBr36AtuCzfg==\""
    );

    assert_eq!(
        req.headers()
            .get("host")
            .expect("host header should be present"),
        "intersight.com"
    );

    assert_eq!(
        req.headers()
            .get("content-type")
            .expect("content-type header should be present"),
        content_type
    );

    assert_eq!(
        req.headers()
            .get("date")
            .expect("date header should be present"),
        req_date
    );
}
