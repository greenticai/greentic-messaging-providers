use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationDescriptor {
    pub name: String,
    pub title: I18nText,
    pub description: I18nText,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactionRule {
    pub path: String,
    pub strategy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DescribePayload {
    pub provider: String,
    pub world: String,
    pub operations: Vec<OperationDescriptor>,
    pub input_schema: SchemaIr,
    pub output_schema: SchemaIr,
    pub config_schema: SchemaIr,
    pub redactions: Vec<RedactionRule>,
    pub schema_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SchemaIr {
    Bool {
        title: I18nText,
        description: I18nText,
    },
    String {
        title: I18nText,
        description: I18nText,
        format: Option<String>,
        secret: bool,
    },
    Object {
        title: I18nText,
        description: I18nText,
        fields: BTreeMap<String, SchemaField>,
        additional_properties: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaField {
    pub required: bool,
    pub schema: SchemaIr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct I18nText {
    pub key: String,
}

/// Question kind — matches `ComponentQaSpec.QuestionKind` in greentic-types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum QuestionKind {
    Text,
    Choice { options: Vec<ChoiceOption> },
    Number,
    Bool,
}

/// Choice option for `QuestionKind::Choice`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChoiceOption {
    pub value: String,
    pub label: I18nText,
}

/// QA question — matches `ComponentQaSpec.Question` in greentic-types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QaQuestionSpec {
    pub id: String,
    pub label: I18nText,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<I18nText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<I18nText>,
    pub kind: QuestionKind,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

/// QA spec — matches `ComponentQaSpec` in greentic-types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QaSpec {
    pub mode: String,
    pub title: I18nText,
    #[serde(default)]
    pub description: Option<I18nText>,
    pub questions: Vec<QaQuestionSpec>,
    #[serde(default)]
    pub defaults: std::collections::BTreeMap<String, serde_json::Value>,
}

pub fn schema_hash(input: &SchemaIr, output: &SchemaIr, config: &SchemaIr) -> String {
    let value = serde_json::json!({
        "input": input,
        "output": output,
        "config": config,
    });
    sha256_hex(&to_canonical_cbor(&value))
}

pub fn canonical_cbor_bytes(value: &impl Serialize) -> Vec<u8> {
    to_canonical_cbor(value)
}

pub fn to_canonical_cbor(value: &impl Serialize) -> Vec<u8> {
    let value = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
    let canonical = canonicalize_json(value);
    let mut out = Vec::new();
    let _ = ciborium::ser::into_writer(&canonical, &mut out);
    out
}

pub fn to_canonical_cbor_allow_floats(value: &impl Serialize) -> Vec<u8> {
    to_canonical_cbor(value)
}

pub fn decode_cbor<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, String> {
    ciborium::de::from_reader(bytes).map_err(|err| err.to_string())
}

pub fn default_en_message_for_key(key: &str) -> String {
    let key = key.trim();
    if key.is_empty() {
        return "Message".to_string();
    }

    let mut words = key
        .split('.')
        .next_back()
        .unwrap_or(key)
        .split('_')
        .filter_map(|token| {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_ascii_lowercase())
            }
        })
        .collect::<Vec<_>>();

    if words.is_empty() {
        return "Message".to_string();
    }

    for word in &mut words {
        match word.as_str() {
            "qa" | "op" | "schema" | "config" | "input" | "output" => {}
            "id" => *word = "ID".to_string(),
            "url" => *word = "URL".to_string(),
            "http" => *word = "HTTP".to_string(),
            "api" => *word = "API".to_string(),
            "ui" => *word = "UI".to_string(),
            "i18n" => *word = "I18N".to_string(),
            _ => {
                let mut chars = word.chars();
                if let Some(first) = chars.next() {
                    *word = format!("{}{}", first.to_ascii_uppercase(), chars.as_str());
                }
            }
        }
    }

    words.join(" ")
}

pub fn default_en_i18n_messages(keys: &[&str]) -> serde_json::Map<String, serde_json::Value> {
    keys.iter()
        .map(|key| {
            (
                (*key).to_string(),
                serde_json::Value::String(default_en_message_for_key(key)),
            )
        })
        .collect::<serde_json::Map<String, serde_json::Value>>()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn canonicalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(canonicalize_json).collect::<Vec<_>>())
        }
        serde_json::Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (key, value) in map {
                sorted.insert(key, canonicalize_json(value));
            }
            let ordered = sorted
                .into_iter()
                .collect::<serde_json::Map<String, serde_json::Value>>();
            serde_json::Value::Object(ordered)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_human_readable_default_i18n_message() {
        assert_eq!(
            default_en_message_for_key("teams.qa.setup.public_base_url"),
            "Public Base URL"
        );
        assert_eq!(
            default_en_message_for_key("telegram.schema.output.message_id.title"),
            "Title"
        );
        assert_eq!(default_en_message_for_key(""), "Message");
    }
}
