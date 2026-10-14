//! Host-neutral HTTP request/response shapes for the ingest op.

#[derive(Debug, Clone, Default)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    /// Raw query string without the leading `?`.
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Self {
            status,
            body: serde_json::to_vec(value).unwrap_or_default(),
        }
    }

    /// An error carrying a stable code only, never detail.
    pub fn error(status: u16, code: &str) -> Self {
        Self::json(status, &serde_json::json!({"error": {"code": code}}))
    }
}

/// A client message to hand to the unit's flow.
#[derive(Debug, Clone, PartialEq)]
pub struct Inbound {
    pub conversation_id: String,
    pub message_id: String,
    pub text: Option<String>,
    pub card: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IngestResult {
    pub response: HttpResponse,
    pub inbound: Option<Inbound>,
}

impl IngestResult {
    pub fn reply(response: HttpResponse) -> Self {
        Self {
            response,
            inbound: None,
        }
    }
}
