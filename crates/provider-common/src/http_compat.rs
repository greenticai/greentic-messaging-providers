//! Operator HTTP format compatibility helpers.
//!
//! The greentic-operator sends `HttpInV1` with query/headers as `Vec<(String,String)>`
//! tuples, but `greentic-types` expects query as `Option<String>` and headers as
//! `Vec<Header>`. These helpers bridge the format mismatch so every provider doesn't
//! need its own copy.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{Header, HttpInV1, HttpOutV1};
use serde_json::{Value, json};

/// Parse the operator's `HttpInV1` format (query as `Vec<[k,v]>`, headers as tuples)
/// into the `greentic-types` `HttpInV1` format.
///
/// Falls back gracefully: if fields are already in the expected format they pass through.
///
/// The `config` field is kept. greentic-start sends deploy-time provider config on
/// `ingest_http` alongside an array-of-pairs `query`, which the native `HttpInV1`
/// cannot deserialize — so this fallback is the only path that request takes, and
/// dropping `config` here left the provider with no config at all.
pub fn parse_operator_http_in(input_json: &[u8]) -> Result<HttpInV1, String> {
    let val: Value = serde_json::from_slice(input_json).map_err(|e| e.to_string())?;
    let method = val
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("POST")
        .to_string();
    let path = val
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("/")
        .to_string();
    let body_b64 = val
        .get("body_b64")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // Operator sends raw bytes as `body: [u8]` (JSON array of numbers);
            // convert to base64 so downstream code can decode uniformly.
            if let Some(Value::Array(arr)) = val.get("body") {
                let bytes: Vec<u8> = arr
                    .iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u8))
                    .collect();
                if !bytes.is_empty() {
                    return STANDARD.encode(&bytes);
                }
            }
            String::new()
        });
    let query = match val.get("query") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(arr)) => {
            let pairs: Vec<String> = arr
                .iter()
                .filter_map(|pair| {
                    if let Value::Array(kv) = pair {
                        let k = kv.first().and_then(|v| v.as_str())?;
                        let v = kv.get(1).and_then(|v| v.as_str()).unwrap_or("");
                        Some(format!("{k}={v}"))
                    } else {
                        None
                    }
                })
                .collect();
            if pairs.is_empty() {
                None
            } else {
                Some(pairs.join("&"))
            }
        }
        _ => None,
    };
    let headers = parse_headers(&val);
    let route_hint = val
        .get("route")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let binding_id = val
        .get("binding_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let config = val.get("config").filter(|v| !v.is_null()).cloned();
    Ok(HttpInV1 {
        method,
        path,
        query,
        headers,
        body_b64,
        route_hint,
        binding_id,
        config,
    })
}

/// Alias of [`parse_operator_http_in`], kept for existing callers (e.g. Email).
///
/// Both functions keep the `config` field; this one predates that and used to be
/// the only one that did.
pub fn parse_operator_http_in_with_config(input_json: &[u8]) -> Result<HttpInV1, String> {
    parse_operator_http_in(input_json)
}

fn parse_headers(val: &Value) -> Vec<Header> {
    match val.get("headers") {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|item| {
                if let Value::Array(kv) = item {
                    let name = kv.first().and_then(|v| v.as_str())?.to_string();
                    let value = kv.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    Some(Header { name, value })
                } else if let Value::Object(map) = item {
                    let name = map.get("name").and_then(|v| v.as_str())?.to_string();
                    let value = map
                        .get("value")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(Header { name, value })
                } else {
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Extract a `(name, value)` header pair from a wrapper-shaped JSON entry,
/// trimmed and rejected if either field is missing or empty post-trim.
///
/// Returns `None` when:
/// - either field is missing or not a string
/// - the value is empty or whitespace-only after trim
///
/// The trim+empty-reject is INSIDE the helper, not the caller's responsibility:
/// header-based identify-instance matching must never admit an empty
/// discriminator (would match an empty admit-table entry — fail-open).
///
/// Accepts both the object form `{"name": "x-...", "value": "..."}` and
/// the tuple form `["x-...", "..."]`.
pub fn header_name_and_value(entry: &Value) -> Option<(&str, &str)> {
    let (name, value) = if let Some(obj) = entry.as_object() {
        let name = obj.get("name")?.as_str()?;
        let value = obj.get("value")?.as_str()?;
        (name, value)
    } else if let Some(arr) = entry.as_array() {
        if arr.len() != 2 {
            return None;
        }
        let name = arr[0].as_str()?;
        let value = arr[1].as_str()?;
        (name, value)
    } else {
        return None;
    };

    let trimmed_value = value.trim();
    if trimmed_value.is_empty() {
        return None;
    }
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return None;
    }
    Some((trimmed_name, trimmed_value))
}

/// Serialize `HttpOutV1` with `"v":1` for operator v0.4.x compatibility.
///
/// Also transforms headers from `[{name, value}]` objects to `[[name, value]]` tuples
/// which the operator expects.
pub fn http_out_v1_bytes(out: &HttpOutV1) -> Vec<u8> {
    let mut val = serde_json::to_value(out).unwrap_or(Value::Null);
    if let Some(map) = val.as_object_mut() {
        map.insert("v".to_string(), json!(1));
        // Transform headers from [{name, value}] to [[name, value]] for operator compat
        if let Some(Value::Array(headers)) = map.get("headers") {
            let tuple_headers: Vec<Value> = headers
                .iter()
                .filter_map(|h| {
                    let name = h.get("name").and_then(|v| v.as_str())?;
                    let value = h.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    Some(json!([name, value]))
                })
                .collect();
            map.insert("headers".to_string(), Value::Array(tuple_headers));
        }
    }
    serde_json::to_vec(&val).unwrap_or_default()
}

/// Build an error response in HttpOutV1 format.
pub fn http_out_error(status: u16, message: &str) -> Vec<u8> {
    let out = HttpOutV1 {
        status,
        headers: Vec::new(),
        body_b64: STANDARD.encode(message.as_bytes()),
        events: Vec::new(),
    };
    http_out_v1_bytes(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_operator_format_with_tuple_query() {
        let input = serde_json::to_vec(&json!({
            "method": "GET",
            "path": "/webhook",
            "body_b64": "",
            "query": [["hub.mode", "subscribe"], ["hub.challenge", "test123"]],
            "headers": [["content-type", "application/json"]],
        }))
        .unwrap();
        let req = parse_operator_http_in(&input).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/webhook");
        assert_eq!(
            req.query.as_deref(),
            Some("hub.mode=subscribe&hub.challenge=test123")
        );
        assert_eq!(req.headers.len(), 1);
        assert_eq!(req.headers[0].name, "content-type");
        assert!(req.config.is_none());
    }

    #[test]
    fn parse_operator_format_with_config() {
        let input = serde_json::to_vec(&json!({
            "method": "POST",
            "path": "/notifications",
            "body_b64": "",
            "query": [],
            "headers": [],
            "config": {"tenant_id": "abc"},
        }))
        .unwrap();
        let req = parse_operator_http_in_with_config(&input).unwrap();
        assert!(req.config.is_some());
        assert_eq!(req.config.unwrap()["tenant_id"], "abc");
    }

    /// The exact shape greentic-start sends on `invoke(op="ingest_http")`: query and
    /// headers as arrays of pairs, plus the deploy-time provider config. The native
    /// `HttpInV1` rejects the array query, so this parser is the only one that sees
    /// the request — and the config has to survive it.
    #[test]
    fn start_wire_shape_keeps_config() {
        let input = serde_json::to_vec(&json!({
            "method": "POST",
            "path": "/v1/messaging/webchat/acme/v3/directline/conversations",
            "query": [["tenant", "acme"], ["team", "_"]],
            "headers": [["content-type", "application/json"], ["authorization", "Bearer t"]],
            "body_b64": "",
            "route": null,
            "binding_id": null,
            "config": {"auto_start_on_open": false, "oauth_enabled_b64": "dHJ1ZQ=="},
        }))
        .unwrap();
        assert!(
            serde_json::from_slice::<HttpInV1>(&input).is_err(),
            "the native parser must reject this shape, or the fallback is not what runs"
        );

        let req = parse_operator_http_in(&input).unwrap();
        assert_eq!(req.query.as_deref(), Some("tenant=acme&team=_"));
        assert_eq!(req.headers.len(), 2);
        assert_eq!(req.headers[1].name, "authorization");
        assert_eq!(
            req.config,
            Some(json!({"auto_start_on_open": false, "oauth_enabled_b64": "dHJ1ZQ=="}))
        );
    }

    #[test]
    fn absent_or_null_config_is_none() {
        let absent = serde_json::to_vec(&json!({"method": "POST", "path": "/"})).unwrap();
        assert!(parse_operator_http_in(&absent).unwrap().config.is_none());

        let null =
            serde_json::to_vec(&json!({"method": "POST", "path": "/", "config": null})).unwrap();
        assert!(parse_operator_http_in(&null).unwrap().config.is_none());
    }

    #[test]
    fn both_entry_points_agree_on_config() {
        let input = serde_json::to_vec(&json!({
            "method": "POST",
            "path": "/notifications",
            "query": [],
            "config": {"tenant_id": "abc"},
        }))
        .unwrap();
        let plain = parse_operator_http_in(&input).unwrap();
        let with_config = parse_operator_http_in_with_config(&input).unwrap();
        assert_eq!(plain.config, Some(json!({"tenant_id": "abc"})));
        assert_eq!(plain.config, with_config.config);
    }

    #[test]
    fn parse_string_query_passthrough() {
        let input = serde_json::to_vec(&json!({
            "method": "GET",
            "path": "/webhook",
            "body_b64": "",
            "query": "foo=bar&baz=qux",
            "config": {"tenant_id": "abc"},
        }))
        .unwrap();
        let req = parse_operator_http_in(&input).unwrap();
        assert_eq!(req.query.as_deref(), Some("foo=bar&baz=qux"));
        assert_eq!(req.config, Some(json!({"tenant_id": "abc"})));
    }

    #[test]
    fn parse_object_headers() {
        let input = serde_json::to_vec(&json!({
            "method": "POST",
            "path": "/",
            "body_b64": "",
            "headers": [{"name": "Authorization", "value": "Bearer tok"}],
        }))
        .unwrap();
        let req = parse_operator_http_in(&input).unwrap();
        assert_eq!(req.headers.len(), 1);
        assert_eq!(req.headers[0].name, "Authorization");
        assert_eq!(req.headers[0].value, "Bearer tok");
    }

    #[test]
    fn parse_operator_body_as_byte_array() {
        // The operator sends body as Vec<u8> (JSON array of numbers),
        // not as body_b64 string. Verify our parser handles this.
        let webhook = r#"{"update_id":123,"message":{"chat":{"id":999},"text":"hello"}}"#;
        let body_bytes: Vec<u8> = webhook.bytes().collect();
        let input = serde_json::to_vec(&json!({
            "method": "POST",
            "path": "/webhook",
            "body": body_bytes,
            "headers": [],
            "query": [],
        }))
        .unwrap();
        let req = parse_operator_http_in(&input).unwrap();
        assert!(
            !req.body_b64.is_empty(),
            "body_b64 should be populated from body array"
        );
        let decoded = STANDARD.decode(&req.body_b64).unwrap();
        let val: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(val["update_id"], 123);
        assert_eq!(val["message"]["chat"]["id"], 999);
    }

    #[test]
    fn http_out_v1_bytes_injects_v_and_transforms_headers() {
        let out = HttpOutV1 {
            status: 200,
            headers: vec![Header {
                name: "Content-Type".into(),
                value: "application/json".into(),
            }],
            body_b64: STANDARD.encode(b"ok"),
            events: Vec::new(),
        };
        let bytes = http_out_v1_bytes(&out);
        let val: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val["v"], 1);
        assert_eq!(val["headers"][0][0], "Content-Type");
        assert_eq!(val["headers"][0][1], "application/json");
    }

    #[test]
    fn http_out_error_returns_valid_json() {
        let bytes = http_out_error(400, "bad request");
        let val: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val["status"], 400);
        assert_eq!(val["v"], 1);
    }

    // ── header_name_and_value contract tests ──

    #[test]
    fn header_name_value_object_normal() {
        let entry = json!({"name": "x-bot-token", "value": "tok123"});
        assert_eq!(
            header_name_and_value(&entry),
            Some(("x-bot-token", "tok123"))
        );
    }

    #[test]
    fn header_name_value_object_missing_value() {
        let entry = json!({"name": "x-bot-token"});
        assert!(header_name_and_value(&entry).is_none());
    }

    #[test]
    fn header_name_value_object_empty_value() {
        let entry = json!({"name": "x-bot-token", "value": ""});
        assert!(header_name_and_value(&entry).is_none());
    }

    #[test]
    fn header_name_value_object_whitespace_value() {
        let entry = json!({"name": "x-bot-token", "value": "   "});
        assert!(header_name_and_value(&entry).is_none());
    }

    #[test]
    fn header_name_value_object_trims_value() {
        let entry = json!({"name": "x-bot-token", "value": "  tok  "});
        assert_eq!(header_name_and_value(&entry), Some(("x-bot-token", "tok")));
    }

    #[test]
    fn header_name_value_object_empty_name() {
        let entry = json!({"name": "", "value": "tok"});
        assert!(header_name_and_value(&entry).is_none());
    }

    #[test]
    fn header_name_value_tuple_normal() {
        let entry = json!(["x-bot-token", "tok456"]);
        assert_eq!(
            header_name_and_value(&entry),
            Some(("x-bot-token", "tok456"))
        );
    }

    #[test]
    fn header_name_value_tuple_one_element() {
        let entry = json!(["x-bot-token"]);
        assert!(header_name_and_value(&entry).is_none());
    }

    #[test]
    fn header_name_value_tuple_three_elements() {
        let entry = json!(["x-bot-token", "v", "extra"]);
        assert!(header_name_and_value(&entry).is_none());
    }

    #[test]
    fn header_name_value_tuple_empty_value() {
        let entry = json!(["x-bot-token", ""]);
        assert!(header_name_and_value(&entry).is_none());
    }

    #[test]
    fn header_name_value_tuple_trims_value() {
        let entry = json!(["x-bot-token", "  tok  "]);
        assert_eq!(header_name_and_value(&entry), Some(("x-bot-token", "tok")));
    }

    #[test]
    fn header_name_value_non_object_non_array() {
        assert!(header_name_and_value(&json!("just a string")).is_none());
        assert!(header_name_and_value(&json!(42)).is_none());
    }

    #[test]
    fn header_name_value_object_non_string_name() {
        let entry = json!({"name": 42, "value": "tok"});
        assert!(header_name_and_value(&entry).is_none());
    }
}
