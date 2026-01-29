use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub use greentic_types::ChannelMessageEnvelope;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HttpInV1 {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub headers: Vec<Header>,
    #[serde(default)]
    pub body_b64: String,
    #[serde(default)]
    pub route_hint: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HttpOutV1 {
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<Header>,
    #[serde(default)]
    pub body_b64: String,
    #[serde(default)]
    pub events: Vec<ChannelMessageEnvelope>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderPlanInV1 {
    pub message: ChannelMessageEnvelope,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderPlanOutV1 {
    pub plan_json: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderPayloadV1 {
    pub content_type: String,
    #[serde(default)]
    pub body_b64: String,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncodeInV1 {
    pub message: ChannelMessageEnvelope,
    pub plan: RenderPlanInV1,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SendPayloadInV1 {
    pub provider_type: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
    pub payload: ProviderPayloadV1,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SendPayloadResultV1 {
    pub ok: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionEnsureInV1 {
    pub v: u8,
    pub provider: String,
    #[serde(default)]
    pub tenant_hint: Option<String>,
    #[serde(default)]
    pub team_hint: Option<String>,
    #[serde(default)]
    pub binding_id: Option<String>,
    pub resource: String,
    #[serde(default)]
    pub change_types: Vec<String>,
    pub notification_url: String,
    #[serde(default)]
    pub expiration_target_unix_ms: Option<u64>,
    #[serde(default)]
    pub client_state: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionEnsureOutV1 {
    pub v: u8,
    pub subscription_id: String,
    pub expiration_unix_ms: u64,
    pub resource: String,
    pub change_types: Vec<String>,
    #[serde(default)]
    pub client_state: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub binding_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionRenewInV1 {
    pub v: u8,
    pub provider: String,
    pub subscription_id: String,
    pub expiration_target_unix_ms: u64,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionRenewOutV1 {
    pub v: u8,
    pub subscription_id: String,
    pub expiration_unix_ms: u64,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionDeleteInV1 {
    pub v: u8,
    pub provider: String,
    pub subscription_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionDeleteOutV1 {
    pub v: u8,
    pub subscription_id: String,
}
