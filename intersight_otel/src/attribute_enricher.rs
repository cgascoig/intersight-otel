use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use intersight_api::Client;
use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, KeyValue};
use tokio::sync::Mutex;

use crate::config::AttributeEnricherConfig;
use crate::intersight_poller::IntersightMetricBatch;

const CACHE_TTL_SECS: u64 = 3600;
const CACHE_JITTER_SECS: u64 = 300;

struct CacheEntry {
    value: Option<HashMap<String, String>>,
    expires_at: Instant,
}

fn jitter_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 % CACHE_JITTER_SECS)
        .unwrap_or(0)
}

pub struct AttributeEnricher {
    config: AttributeEnricherConfig,
    cache: Mutex<HashMap<String, CacheEntry>>,
    client: Client,
    compiled_regex: Option<regex::Regex>,
}

impl AttributeEnricher {
    pub fn new(config: AttributeEnricherConfig, client: Client) -> Self {
        let compiled_regex = config.source_value_regex.as_ref().map(|pattern| {
            regex::Regex::new(pattern).unwrap_or_else(|e| {
                panic!(
                    "Enricher '{}': invalid source_value_regex '{}': {}",
                    config.name, pattern, e
                )
            })
        });
        AttributeEnricher {
            config,
            cache: Mutex::new(HashMap::new()),
            client,
            compiled_regex,
        }
    }

    pub async fn enrich_batch(&self, batch: &mut IntersightMetricBatch) {
        trace!("enrich_batch called");
        let mut enriched = 0usize;
        let mut skipped = 0usize;

        for resource in batch {
            // Find the source attribute value
            let source_value = resource.attributes.iter().find_map(|kv| {
                if kv.key == self.config.source_attribute {
                    if let Some(AnyValue {
                        value: Some(any_value::Value::StringValue(s)),
                    }) = &kv.value
                    {
                        return Some(s.clone());
                    }
                }
                None
            });

            let Some(source_value) = source_value else {
                skipped += 1;
                continue;
            };

            let query_value = match &self.compiled_regex {
                Some(re) => match apply_regex(&source_value, re) {
                    Some(v) => v,
                    None => {
                        warn!(
                            "Enricher '{}': regex did not match source value '{}'",
                            self.config.name, source_value
                        );
                        skipped += 1;
                        continue;
                    }
                },
                None => source_value.clone(),
            };

            let result = self.lookup(&query_value).await;

            if let Some(result_map) = result {
                for (attr_key, attr_val) in result_map {
                    resource.attributes.push(KeyValue {
                        key: attr_key,
                        value: Some(AnyValue {
                            value: Some(any_value::Value::StringValue(attr_val)),
                        }),
                    });
                }
                enriched += 1;
            } else {
                skipped += 1;
            }
        }
        debug!(
            "Enricher '{}': enriched {}/{} resources",
            self.config.name,
            enriched,
            enriched + skipped
        );
    }

    async fn lookup(&self, source_value: &str) -> Option<HashMap<String, String>> {
        {
            let cache = self.cache.lock().await;
            if let Some(entry) = cache.get(source_value) {
                if Instant::now() < entry.expires_at {
                    return entry.value.clone();
                }
                // Entry expired; fall through to re-lookup
            }
        }

        let result = self.do_lookup(source_value).await;

        let expires_at = Instant::now() + Duration::from_secs(CACHE_TTL_SECS + jitter_secs());
        let mut cache = self.cache.lock().await;
        cache.insert(
            source_value.to_string(),
            CacheEntry {
                value: result.clone(),
                expires_at,
            },
        );

        result
    }

    async fn do_lookup(&self, source_value: &str) -> Option<HashMap<String, String>> {
        let path = self.config.query_template.replace("{value}", source_value);
        let response = match self.client.get(&path).await {
            Ok(r) => r,
            Err(err) => {
                warn!(
                    "Enricher '{}': API call failed for value '{}': {}",
                    self.config.name, source_value, err
                );
                return None;
            }
        };

        let mut result = HashMap::new();
        for mapping in &self.config.result_mappings {
            if let Some(value_str) = extract_field(&response, &mapping.result_field) {
                result.insert(mapping.result_attribute.clone(), value_str);
            } else {
                warn!(
                    "Enricher '{}': field '{}' not found in response for value '{}'",
                    self.config.name, mapping.result_field, source_value
                );
            }
        }
        Some(result)
    }
}

fn apply_regex(value: &str, re: &regex::Regex) -> Option<String> {
    let captures = re.captures(value)?;
    let m = captures.get(1).or_else(|| captures.get(0))?;
    Some(m.as_str().to_string())
}

fn extract_field(response: &serde_json::Value, field: &str) -> Option<String> {
    let path = match serde_json_path::JsonPath::parse(field) {
        Ok(p) => p,
        Err(e) => {
            warn!("Invalid JSONPath '{}': {}", field, e);
            return None;
        }
    };
    let value = path.query(response).first()?;
    Some(match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

pub fn build_enricher_map(
    configs: &[AttributeEnricherConfig],
    client: &Client,
) -> HashMap<String, Arc<AttributeEnricher>> {
    configs
        .iter()
        .map(|c| {
            (
                c.name.clone(),
                Arc::new(AttributeEnricher::new(c.clone(), client.clone())),
            )
        })
        .collect()
}

pub fn resolve_enrichers(
    names: &[String],
    map: &HashMap<String, Arc<AttributeEnricher>>,
) -> Vec<Arc<AttributeEnricher>> {
    names
        .iter()
        .filter_map(|name| {
            let e = map.get(name).cloned();
            if e.is_none() {
                warn!("Enricher '{}' not found in config", name);
            }
            e
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ResultMappingConfig;
    use crate::intersight_poller::IntersightResourceMetrics;
    use serde_json::json;

    // Inline test PEM key (same as used in intersight_api tests)
    const TEST_KEY_ID: &str =
        "59c84e4a16267c0001c23428/59cc595416267c0001a0dfc7/62b3ba347564612d3198f5b1";
    //NOSONAR
    const TEST_PEM: &[u8] = b"-----BEGIN RSA PRIVATE KEY-----
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
-----END RSA PRIVATE KEY-----";

    fn test_client() -> Client {
        intersight_api::config::Config::new()
            .with_key_id(TEST_KEY_ID)
            .with_key_bytes(TEST_PEM)
            .build_client()
            .expect("failed to build test client")
    }

    fn make_config(name: &str) -> AttributeEnricherConfig {
        AttributeEnricherConfig {
            name: name.to_string(),
            source_attribute: "some.attribute".to_string(),
            source_value_regex: None,
            query_template: "api/v1/thing/{value}".to_string(),
            result_mappings: vec![ResultMappingConfig {
                result_field: "$.Name".to_string(),
                result_attribute: "thing.name".to_string(),
            }],
        }
    }

    fn make_enricher(name: &str) -> Arc<AttributeEnricher> {
        Arc::new(AttributeEnricher::new(make_config(name), test_client()))
    }

    fn string_attribute(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(value.to_string())),
            }),
        }
    }

    fn attribute_value<'a>(resource: &'a IntersightResourceMetrics, key: &str) -> Option<&'a str> {
        resource.attributes.iter().find_map(|kv| {
            if kv.key != key {
                return None;
            }
            match &kv.value {
                Some(AnyValue {
                    value: Some(any_value::Value::StringValue(value)),
                }) => Some(value.as_str()),
                _ => None,
            }
        })
    }

    // --- extract_field tests ---

    #[test]
    fn test_extract_field_top_level_string() {
        let response = json!({"Name": "my-cluster", "Count": 5});
        assert_eq!(
            extract_field(&response, "$.Name"),
            Some("my-cluster".to_string())
        );
    }

    #[test]
    fn test_extract_field_top_level_number() {
        let response = json!({"Count": 42});
        assert_eq!(extract_field(&response, "$.Count"), Some("42".to_string()));
    }

    #[test]
    fn test_extract_field_top_level_missing() {
        let response = json!({"Name": "foo"});
        assert_eq!(extract_field(&response, "$.Missing"), None);
    }

    #[test]
    fn test_extract_field_nested_string() {
        let response = json!({"Nested": {"Version": "1.2.3"}});
        assert_eq!(
            extract_field(&response, "$.Nested.Version"),
            Some("1.2.3".to_string())
        );
    }

    #[test]
    fn test_extract_field_nested_number() {
        let response = json!({"Stats": {"Count": 99}});
        assert_eq!(
            extract_field(&response, "$.Stats.Count"),
            Some("99".to_string())
        );
    }

    #[test]
    fn test_extract_field_nested_missing() {
        let response = json!({"Nested": {"Version": "1.2.3"}});
        assert_eq!(extract_field(&response, "$.Nested.DoesNotExist"), None);
    }

    #[test]
    fn test_extract_field_converts_json_scalar_values() {
        let response = json!({"Enabled": true, "Nothing": null});
        assert_eq!(
            extract_field(&response, "$.Enabled"),
            Some("true".to_string())
        );
        assert_eq!(
            extract_field(&response, "$.Nothing"),
            Some("null".to_string())
        );
    }

    #[test]
    fn test_extract_field_serializes_array_and_object_values() {
        let response = json!({"Items": [1, "two"], "Details": {"Name": "thing"}});
        assert_eq!(
            extract_field(&response, "$.Items"),
            Some(r#"[1,"two"]"#.to_string())
        );
        assert_eq!(
            extract_field(&response, "$.Details"),
            Some(r#"{"Name":"thing"}"#.to_string())
        );
    }

    #[test]
    fn test_extract_field_returns_first_json_path_match() {
        let response = json!({"Items": [{"Name": "first"}, {"Name": "second"}]});
        assert_eq!(
            extract_field(&response, "$.Items[*].Name"),
            Some("first".to_string())
        );
    }

    #[test]
    fn test_extract_field_invalid_json_path() {
        let response = json!({"Name": "thing"});
        assert_eq!(extract_field(&response, "$["), None);
    }

    // --- enrich_batch tests ---

    #[tokio::test]
    async fn test_enrich_batch_skips_missing_source_attribute_without_lookup() {
        let enricher = make_enricher("missing-source");
        let mut batch = vec![IntersightResourceMetrics {
            attributes: vec![string_attribute("other.attribute", "value")],
            ..Default::default()
        }];

        enricher.enrich_batch(&mut batch).await;

        assert_eq!(batch[0].attributes.len(), 1);
        assert!(enricher.cache.lock().await.is_empty());
    }

    #[tokio::test]
    async fn test_enrich_batch_skips_non_string_source_without_lookup() {
        let enricher = make_enricher("non-string-source");
        let mut batch = vec![IntersightResourceMetrics {
            attributes: vec![KeyValue {
                key: "some.attribute".to_string(),
                value: Some(AnyValue {
                    value: Some(any_value::Value::IntValue(42)),
                }),
            }],
            ..Default::default()
        }];

        enricher.enrich_batch(&mut batch).await;

        assert_eq!(batch[0].attributes.len(), 1);
        assert!(enricher.cache.lock().await.is_empty());
    }

    #[tokio::test]
    async fn test_enrich_batch_skips_regex_non_match_without_lookup() {
        let mut config = make_config("regex-non-match");
        config.source_value_regex = Some(r"^moid:(\d+)$".to_string());
        let enricher = AttributeEnricher::new(config, test_client());
        let mut batch = vec![IntersightResourceMetrics {
            attributes: vec![string_attribute("some.attribute", "not-a-moid")],
            ..Default::default()
        }];

        enricher.enrich_batch(&mut batch).await;

        assert_eq!(batch[0].attributes.len(), 1);
        assert!(enricher.cache.lock().await.is_empty());
    }

    #[tokio::test]
    async fn test_enrich_batch_uses_regex_capture_as_cache_key() {
        let mut config = make_config("regex-cache-key");
        config.source_value_regex = Some(r"^moid:(\d+)$".to_string());
        let enricher = AttributeEnricher::new(config, test_client());
        enricher.cache.lock().await.insert(
            "123".to_string(),
            CacheEntry {
                value: Some(HashMap::from([(
                    "thing.name".to_string(),
                    "cached-name".to_string(),
                )])),
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );
        let mut batch = vec![IntersightResourceMetrics {
            attributes: vec![string_attribute("some.attribute", "moid:123")],
            ..Default::default()
        }];

        enricher.enrich_batch(&mut batch).await;

        assert_eq!(
            attribute_value(&batch[0], "thing.name"),
            Some("cached-name")
        );
        assert_eq!(enricher.cache.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn test_enrich_batch_repeated_positive_cache_hits_enrich_each_batch() {
        let enricher = make_enricher("positive-cache-hit");
        enricher.cache.lock().await.insert(
            "source-1".to_string(),
            CacheEntry {
                value: Some(HashMap::from([(
                    "thing.name".to_string(),
                    "cached-name".to_string(),
                )])),
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );
        let mut first_batch = vec![IntersightResourceMetrics {
            attributes: vec![string_attribute("some.attribute", "source-1")],
            ..Default::default()
        }];
        let mut second_batch = vec![IntersightResourceMetrics {
            attributes: vec![string_attribute("some.attribute", "source-1")],
            ..Default::default()
        }];

        enricher.enrich_batch(&mut first_batch).await;
        enricher.enrich_batch(&mut second_batch).await;

        assert_eq!(
            attribute_value(&first_batch[0], "thing.name"),
            Some("cached-name")
        );
        assert_eq!(
            attribute_value(&second_batch[0], "thing.name"),
            Some("cached-name")
        );
        let cache = enricher.cache.lock().await;
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache["source-1"].value.as_ref().unwrap()["thing.name"],
            "cached-name"
        );
    }

    #[tokio::test]
    async fn test_enrich_batch_negative_cache_hit_skips_without_lookup() {
        let enricher = make_enricher("negative-cache-hit");
        enricher.cache.lock().await.insert(
            "missing".to_string(),
            CacheEntry {
                value: None,
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );
        let mut batch = vec![IntersightResourceMetrics {
            attributes: vec![string_attribute("some.attribute", "missing")],
            ..Default::default()
        }];

        enricher.enrich_batch(&mut batch).await;

        assert_eq!(batch[0].attributes.len(), 1);
        let cache = enricher.cache.lock().await;
        assert_eq!(cache.len(), 1);
        assert!(cache["missing"].value.is_none());
    }

    // --- resolve_enrichers tests ---

    #[test]
    fn test_resolve_enrichers_all_found() {
        let mut map = HashMap::new();
        map.insert("alpha".to_string(), make_enricher("alpha"));
        map.insert("beta".to_string(), make_enricher("beta"));

        let names = vec!["alpha".to_string(), "beta".to_string()];
        let resolved = resolve_enrichers(&names, &map);
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn test_resolve_enrichers_unknown_skipped() {
        let mut map = HashMap::new();
        map.insert("alpha".to_string(), make_enricher("alpha"));

        let names = vec!["alpha".to_string(), "nonexistent".to_string()];
        let resolved = resolve_enrichers(&names, &map);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].config.name, "alpha");
    }

    #[test]
    fn test_resolve_enrichers_empty_names() {
        let mut map = HashMap::new();
        map.insert("alpha".to_string(), make_enricher("alpha"));

        let resolved = resolve_enrichers(&[], &map);
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_resolve_enrichers_empty_map() {
        let map: HashMap<String, Arc<AttributeEnricher>> = HashMap::new();
        let names = vec!["alpha".to_string()];
        let resolved = resolve_enrichers(&names, &map);
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_resolve_enrichers_preserves_request_order_and_duplicates() {
        let mut map = HashMap::new();
        map.insert("alpha".to_string(), make_enricher("alpha"));
        map.insert("beta".to_string(), make_enricher("beta"));

        let names = vec!["beta".to_string(), "alpha".to_string(), "beta".to_string()];
        let resolved = resolve_enrichers(&names, &map);
        let resolved_names: Vec<_> = resolved.iter().map(|e| e.config.name.as_str()).collect();

        assert_eq!(resolved_names, ["beta", "alpha", "beta"]);
        assert!(Arc::ptr_eq(&resolved[0], &resolved[2]));
    }

    // --- build_enricher_map tests ---

    #[test]
    fn test_build_enricher_map_keys() {
        let configs = vec![
            AttributeEnricherConfig {
                name: "enricher_a".to_string(),
                source_attribute: "a".to_string(),
                source_value_regex: None,
                query_template: "api/v1/a/{value}".to_string(),
                result_mappings: vec![],
            },
            AttributeEnricherConfig {
                name: "enricher_b".to_string(),
                source_attribute: "b".to_string(),
                source_value_regex: None,
                query_template: "api/v1/b/{value}".to_string(),
                result_mappings: vec![],
            },
        ];
        let client = test_client();
        let map = build_enricher_map(&configs, &client);
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("enricher_a"));
        assert!(map.contains_key("enricher_b"));
    }

    #[test]
    fn test_build_enricher_map_empty() {
        let client = test_client();
        let map = build_enricher_map(&[], &client);
        assert!(map.is_empty());
    }

    #[test]
    fn test_build_enricher_map_duplicate_name_uses_last_config() {
        let mut first = make_config("duplicate");
        first.source_attribute = "first.attribute".to_string();
        let mut second = make_config("duplicate");
        second.source_attribute = "second.attribute".to_string();

        let map = build_enricher_map(&[first, second], &test_client());

        assert_eq!(map.len(), 1);
        assert_eq!(map["duplicate"].config.source_attribute, "second.attribute");
    }

    // --- apply_regex tests ---

    #[test]
    fn test_apply_regex_full_match() {
        let re = regex::Regex::new(r"\d+").unwrap();
        assert_eq!(apply_regex("abc123def", &re), Some("123".to_string()));
    }

    #[test]
    fn test_apply_regex_capture_group() {
        let re = regex::Regex::new(r"foo(\d+)bar").unwrap();
        assert_eq!(apply_regex("foo42bar", &re), Some("42".to_string()));
    }

    #[test]
    fn test_apply_regex_no_match() {
        let re = regex::Regex::new(r"\d+").unwrap();
        assert_eq!(apply_regex("no digits here", &re), None);
    }

    #[test]
    fn test_apply_regex_last_path_segment() {
        let re = regex::Regex::new(r"[^/]+$").unwrap();
        assert_eq!(
            apply_regex("/api/v1/compute/Blades/651a30d276752d35013bc045", &re),
            Some("651a30d276752d35013bc045".to_string())
        );
    }

    #[test]
    fn test_apply_regex_uses_first_capture_group_when_multiple_exist() {
        let re = regex::Regex::new(r"(first)-(second)").unwrap();
        assert_eq!(
            apply_regex("prefix first-second suffix", &re),
            Some("first".to_string())
        );
    }

    #[test]
    fn test_apply_regex_falls_back_to_full_match_when_optional_capture_is_absent() {
        let re = regex::Regex::new(r"(?:prefix-)?(\d+)?value").unwrap();
        assert_eq!(
            apply_regex("prefix-value", &re),
            Some("prefix-value".to_string())
        );
    }

    #[test]
    fn test_apply_regex_preserves_empty_capture() {
        let re = regex::Regex::new(r"^(.*)$").unwrap();
        assert_eq!(apply_regex("", &re), Some(String::new()));
    }
}
