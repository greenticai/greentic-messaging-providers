use std::collections::VecDeque;
use std::io::Read;
use std::sync::{Arc, Mutex};

use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_interfaces_wasmtime::host_helpers::v1::http_client;
use http::{Request, Response as RawResponse};
use serde::Serialize;
use ureq::{Body, agent};

pub type HttpHistory = Arc<Mutex<Vec<HttpCall>>>;
pub type HttpResponseQueue = Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>;

#[derive(Debug, Clone, Copy, Default)]
pub enum HttpMode {
    #[default]
    Mock,
    Real,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpCall {
    pub request: HttpRequest,
    pub response: HttpResponseRecord,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<Header>,
    pub body_b64: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpResponseRecord {
    pub status: u16,
    pub headers: Vec<Header>,
    pub body_b64: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

impl HttpRequest {
    pub fn from_host(req: &http_client::RequestV1_1) -> Self {
        let body_b64 = req.body.as_ref().map(|bytes| STANDARD.encode(bytes));
        let headers = req
            .headers
            .iter()
            .map(|(name, value)| Header {
                name: name.clone(),
                value: value.clone(),
            })
            .collect();
        Self {
            method: req.method.clone(),
            url: req.url.clone(),
            headers,
            body_b64,
        }
    }
}

impl HttpResponseRecord {
    pub fn from_host(resp: &http_client::ResponseV1_1) -> Self {
        let body_b64 = resp.body.as_ref().map(|bytes| STANDARD.encode(bytes));
        let headers = resp
            .headers
            .iter()
            .map(|(name, value)| Header {
                name: name.clone(),
                value: value.clone(),
            })
            .collect();
        Self {
            status: resp.status,
            headers,
            body_b64,
        }
    }
}

pub fn new_history() -> HttpHistory {
    Arc::new(Mutex::new(Vec::new()))
}

#[cfg(test)]
pub fn new_response_queue() -> HttpResponseQueue {
    Arc::new(Mutex::new(VecDeque::new()))
}

const MOCK_RESPONSE_BODY: &[u8] =
    br#"{"ok":true,"result":{"url":"https://example.invalid/mock-webhook"},"description":"mock response"}"#;

#[cfg(test)]
pub fn queue_mock_response(queue: &HttpResponseQueue, status: u16, body: Vec<u8>) {
    if let Ok(mut guard) = queue.lock() {
        guard.push_back((status, body));
    }
}

#[cfg(test)]
pub fn clear_mock_responses(queue: &HttpResponseQueue) {
    if let Ok(mut guard) = queue.lock() {
        guard.clear();
    }
}

pub fn mock_response(queue: Option<&HttpResponseQueue>) -> http_client::ResponseV1_1 {
    let (status, body) = if let Some(queue) = queue {
        if let Ok(mut guard) = queue.lock() {
            guard
                .pop_front()
                .unwrap_or((200, MOCK_RESPONSE_BODY.to_vec()))
        } else {
            (200, MOCK_RESPONSE_BODY.to_vec())
        }
    } else {
        (200, MOCK_RESPONSE_BODY.to_vec())
    };
    http_client::ResponseV1_1 {
        status,
        headers: Vec::new(),
        body: Some(body),
    }
}

pub fn send_real_request(
    req: &http_client::RequestV1_1,
) -> Result<http_client::ResponseV1_1, http_client::HttpClientErrorV1_1> {
    let agent = agent();
    let mut builder = Request::builder().method(req.method.as_str()).uri(&req.url);
    for (name, value) in &req.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let body_bytes = req.body.clone().unwrap_or_default();
    let body_text = String::from_utf8_lossy(&body_bytes);
    println!(
        "real http request {} {} body={}",
        req.method, req.url, body_text
    );
    let request =
        builder
            .body(body_bytes.clone())
            .map_err(|err| http_client::HttpClientErrorV1_1 {
                code: "http_request_build".into(),
                message: err.to_string(),
            })?;
    let response = agent.run(request);
    match response {
        Ok(resp) => build_response(resp),
        Err(err) => Err(http_client::HttpClientErrorV1_1 {
            code: "http_transport_error".into(),
            message: err.to_string(),
        }),
    }
}

fn build_response(
    resp: RawResponse<Body>,
) -> Result<http_client::ResponseV1_1, http_client::HttpClientErrorV1_1> {
    let status = resp.status();
    let headers = resp
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let mut reader = resp.into_body().into_reader();
    let mut body = Vec::new();
    reader
        .read_to_end(&mut body)
        .map_err(|err| http_client::HttpClientErrorV1_1 {
            code: "http_read_error".into(),
            message: err.to_string(),
        })?;
    Ok(http_client::ResponseV1_1 {
        status: status.as_u16(),
        headers,
        body: Some(body),
    })
}
