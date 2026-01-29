use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use greentic_types::{EnvId, MessageMetadata, TenantCtx, TenantId};
use messaging_universal_dto::{
    ChannelMessageEnvelope, EncodeInV1, Header, HttpInV1, HttpOutV1, ProviderPayloadV1,
    RenderPlanInV1, RenderPlanOutV1, SendPayloadInV1, SendPayloadResultV1,
};
use provider_tests::harness::{
    TestHostState, add_wasi_to_linker, add_wasmtime_hosts, component_path, ensure_components_built,
    new_engine,
};
use serde::Deserialize;
use serde_json::{Value, json};
use wasmtime::Store;
use wasmtime::Trap;
use wasmtime::component::{Component, ComponentExportIndex, Instance, Linker, TypedFunc};

macro_rules! provider_bindings {
    ($module:ident, $path:literal, $world:literal) => {
        mod $module {
            wasmtime::component::bindgen!({
                path: $path,
                world: $world,
            });
        }
    };
}

provider_bindings!(
    slack_bindings,
    "../../components/messaging-provider-slack/wit/messaging-provider-slack",
    "messaging-provider-slack"
);
provider_bindings!(
    telegram_bindings,
    "../../components/messaging-provider-telegram/wit/messaging-provider-telegram",
    "messaging-provider-telegram"
);
provider_bindings!(
    teams_bindings,
    "../../components/messaging-provider-teams/wit/messaging-provider-teams",
    "messaging-provider-teams"
);
provider_bindings!(
    webchat_bindings,
    "../../components/messaging-provider-webchat/wit/messaging-provider-webchat",
    "messaging-provider-webchat"
);
provider_bindings!(
    webex_bindings,
    "../../components/messaging-provider-webex/wit/messaging-provider-webex",
    "messaging-provider-webex"
);
provider_bindings!(
    whatsapp_bindings,
    "../../components/messaging-provider-whatsapp/wit/messaging-provider-whatsapp",
    "messaging-provider-whatsapp"
);
provider_bindings!(
    email_bindings,
    "../../components/messaging-provider-email/wit/messaging-provider-email",
    "messaging-provider-email"
);
provider_bindings!(
    dummy_bindings,
    "../../components/messaging-provider-dummy/wit/messaging-provider-dummy",
    "schema-core"
);

#[derive(Clone, Copy, Debug)]
enum ProviderId {
    Slack,
    Telegram,
    Teams,
    Webchat,
    Webex,
    Whatsapp,
    Email,
    Dummy,
}

impl ProviderId {
    fn component_name(&self) -> &'static str {
        match self {
            ProviderId::Slack => "messaging-provider-slack",
            ProviderId::Telegram => "messaging-provider-telegram",
            ProviderId::Teams => "messaging-provider-teams",
            ProviderId::Webchat => "messaging-provider-webchat",
            ProviderId::Webex => "messaging-provider-webex",
            ProviderId::Whatsapp => "messaging-provider-whatsapp",
            ProviderId::Email => "messaging-provider-email",
            ProviderId::Dummy => "messaging-provider-dummy",
        }
    }
}

struct ProviderSpec {
    id: ProviderId,
    provider_type: &'static str,
    fixture: &'static str,
    ingest_supported: bool,
    challenge_fixture: Option<&'static str>,
    challenge_response: Option<&'static str>,
    skip_universal_ops: bool,
}

const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        id: ProviderId::Slack,
        provider_type: "messaging.slack.api",
        fixture: "slack.json",
        ingest_supported: false,
        challenge_fixture: None,
        challenge_response: None,
        skip_universal_ops: true,
    },
    ProviderSpec {
        id: ProviderId::Telegram,
        provider_type: "messaging.telegram.bot",
        fixture: "telegram.json",
        ingest_supported: true,
        challenge_fixture: None,
        challenge_response: None,
        skip_universal_ops: false,
    },
    ProviderSpec {
        id: ProviderId::Teams,
        provider_type: "messaging.teams.graph",
        fixture: "teams.json",
        ingest_supported: true,
        challenge_fixture: None,
        challenge_response: None,
        skip_universal_ops: false,
    },
    ProviderSpec {
        id: ProviderId::Webchat,
        provider_type: "messaging.webchat",
        fixture: "webchat.json",
        ingest_supported: true,
        challenge_fixture: None,
        challenge_response: None,
        skip_universal_ops: false,
    },
    ProviderSpec {
        id: ProviderId::Whatsapp,
        provider_type: "messaging.whatsapp.cloud",
        fixture: "whatsapp.json",
        ingest_supported: true,
        challenge_fixture: Some("whatsapp_challenge.json"),
        challenge_response: Some("verify123"),
        skip_universal_ops: false,
    },
    ProviderSpec {
        id: ProviderId::Webex,
        provider_type: "messaging.webex.bot",
        fixture: "webex.json",
        ingest_supported: false,
        challenge_fixture: None,
        challenge_response: None,
        skip_universal_ops: false,
    },
    ProviderSpec {
        id: ProviderId::Email,
        provider_type: "messaging.email.smtp",
        fixture: "email.json",
        ingest_supported: false,
        challenge_fixture: None,
        challenge_response: None,
        skip_universal_ops: false,
    },
    ProviderSpec {
        id: ProviderId::Dummy,
        provider_type: "messaging.dummy",
        fixture: "dummy.json",
        ingest_supported: true,
        challenge_fixture: None,
        challenge_response: None,
        skip_universal_ops: false,
    },
];

#[derive(Deserialize)]
struct HttpFixture {
    method: String,
    path: String,
    headers: Option<HashMap<String, String>>,
    query: Option<String>,
    route_hint: Option<String>,
    body: Option<Value>,
}

#[derive(Deserialize)]
struct RenderPlanResponse {
    ok: bool,
    plan: Option<RenderPlanOutV1>,
}

#[derive(Deserialize)]
struct EncodeResponse {
    ok: bool,
    payload: Option<ProviderPayloadV1>,
}

struct ProviderHarness {
    _instance: Instance,
    store: Store<TestHostState>,
    invoke: TypedFunc<(String, Vec<u8>), (Vec<u8>,)>,
}

impl ProviderHarness {
    fn new(spec: &ProviderSpec) -> Result<Self> {
        let engine = new_engine();
        let component_file = component_path(spec.id.component_name());
        log_component_artifact(&component_file);
        let component = Component::from_file(&engine, &component_file).context("load component")?;
        let mut linker = Linker::new(&engine);
        add_wasi_to_linker(&mut linker);
        add_wasmtime_hosts(&mut linker)?;
        let mut store = Store::new(&engine, TestHostState::with_default_secrets());
        let instance = linker
            .instantiate(&mut store, &component)
            .map_err(|e| instantiate_error(spec, e))?;
        let api_index: ComponentExportIndex = instance
            .get_export_index(
                &mut store,
                None,
                "greentic:provider-schema-core/schema-core-api@1.0.0",
            )
            .context("schema-core export index")?;
        let invoke_index = instance
            .get_export_index(&mut store, Some(&api_index), "invoke")
            .context("invoke export index")?;
        let invoke = instance
            .get_typed_func(&mut store, invoke_index)
            .context("typed invoke func")?;
        Ok(Self {
            _instance: instance,
            store,
            invoke,
        })
    }

    fn call(&mut self, op: &str, payload: Vec<u8>) -> Result<Vec<u8>> {
        let (result,) = self
            .invoke
            .call(&mut self.store, (op.to_string(), payload))
            .map_err(|err| {
                if let Some(trap) = err.downcast_ref::<Trap>() {
                    eprintln!("trap trace: {:?}", trap);
                }
                err
            })
            .context(format!("invoke {op}"))?;
        self.invoke
            .post_return(&mut self.store)
            .context("post return")?;
        Ok(result)
    }
}

fn log_component_artifact(path: &Path) {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let metadata = fs::metadata(&canonical).ok();
    let (len, mtime) = if let Some(meta) = metadata {
        let len = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        (len, mtime)
    } else {
        (0, 0)
    };
    eprintln!(
        "[provider-tests] Instantiating component: {} (bytes={}, mtime={})",
        canonical.display(),
        len,
        mtime
    );
}

fn instantiate_error<E: std::fmt::Display>(spec: &ProviderSpec, source: E) -> Error {
    let text = source.to_string();
    let hint = if text.contains("expected 2-tuple") {
        Some(
            "component still imports legacy http-client signature (request, ctx); rebuild against http-client with RequestOptions (3-tuple)",
        )
    } else if text.contains("expected 3-tuple") {
        Some(
            "harness exposes 3-tuple signature (request, requestOptions, ctx); rebuild provider so it targets the new contract",
        )
    } else if text.contains("matching implementation was not found") {
        Some(
            "legacy http-client import is unsupported; rebuild the provider so it imports the 3-tuple signature (request, requestOptions, ctx)",
        )
    } else {
        None
    };
    let detail = if let Some(h) = hint {
        format!("{} ({})", text, h)
    } else {
        text
    };
    Error::msg(format!(
        "failed to instantiate {} ({}): {}",
        spec.id.component_name(),
        spec.provider_type,
        detail
    ))
}

fn fixtures_root() -> PathBuf {
    ensure_components_built();
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root.pop();
    root.push("tests/fixtures/universal");
    root
}

fn load_http_fixture(name: &str) -> Result<HttpFixture> {
    let path = fixtures_root().join(name);
    let raw = fs::read_to_string(&path).context("load fixture")?;
    Ok(serde_json::from_str(&raw).context("parse fixture")?)
}

fn http_input_from_fixture(fixture: HttpFixture) -> HttpInV1 {
    let headers = fixture
        .headers
        .unwrap_or_default()
        .into_iter()
        .map(|(name, value)| Header { name, value })
        .collect();
    let body_bytes = fixture
        .body
        .map(|body| serde_json::to_vec(&body).unwrap_or_default())
        .unwrap_or_default();
    HttpInV1 {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers,
        body_b64: STANDARD.encode(&body_bytes),
        route_hint: fixture.route_hint,
    }
}

fn build_envelope(id: ProviderId) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("default env");
    let tenant = TenantId::try_from("default").expect("default tenant");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    let channel = match id {
        ProviderId::Slack => "slack",
        ProviderId::Telegram => "telegram",
        ProviderId::Teams => "teams",
        ProviderId::Webchat => "webchat",
        ProviderId::Webex => "webex",
        ProviderId::Whatsapp => "whatsapp",
        ProviderId::Email => "email",
        ProviderId::Dummy => "dummy",
    };
    ChannelMessageEnvelope {
        id: format!("{channel}-envelope"),
        tenant: TenantCtx::new(env, tenant),
        channel: channel.to_string(),
        session_id: channel.to_string(),
        reply_scope: None,
        user_id: Some("universal-user".to_string()),
        correlation_id: None,
        text: Some(format!("universal {} message", channel)),
        attachments: Vec::new(),
        metadata,
    }
}

fn send_payload_body(id: ProviderId) -> Value {
    match id {
        ProviderId::Slack => json!({
            "to": {"kind": "channel", "id": "C_UNIVERSAL"},
            "text": "universal slack",
            "config": {"default_channel": "C_UNIVERSAL"}
        }),
        ProviderId::Telegram => json!({
            "chat_id": "922337",
            "text": "universal telegram",
            "config": {"default_chat_id": "922337"}
        }),
        ProviderId::Teams => json!({
            "text": "universal teams",
            "team_id": "team-universal",
            "channel_id": "channel-universal",
            "config": {
                "tenant_id": "tenant-universal",
                "client_id": "client-universal",
                "team_id": "team-universal",
                "channel_id": "channel-universal"
            }
        }),
        ProviderId::Webchat => json!({
            "text": "universal webchat",
            "route": "universal-route",
            "tenant_channel_id": "tenant-channel",
            "config": {
                "route": "universal-route",
                "tenant_channel_id": "tenant-channel",
                "mode": "universal-mode",
                "base_url": "https://webchat.example"
            }
        }),
        ProviderId::Webex => json!({
            "text": "universal webex",
            "to": {"kind": "room", "id": "room-universal"},
            "config": {"default_room_id": "room-universal"}
        }),
        ProviderId::Whatsapp => json!({
            "text": "universal whatsapp",
            "to": {"kind": "user", "id": "whatsapp-user"},
            "config": {"phone_number_id": "phone-universal"}
        }),
        ProviderId::Email => json!({
            "to": "recipient@example.com",
            "subject": "universal email",
            "body": "hello universal",
            "config": {
                "host": "smtp.example",
                "username": "user",
                "from_address": "sender@example.com"
            }
        }),
        ProviderId::Dummy => json!({
            "text": "universal dummy"
        }),
    }
}

fn send_payload_in(spec: &ProviderSpec) -> Result<Vec<u8>> {
    let body = send_payload_body(spec.id);
    let body_bytes = serde_json::to_vec(&body)?;
    let payload = ProviderPayloadV1 {
        content_type: "application/json".to_string(),
        body_b64: STANDARD.encode(&body_bytes),
        metadata: HashMap::new(),
    };
    let payload_in = SendPayloadInV1 {
        provider_type: spec.provider_type.to_string(),
        tenant_id: None,
        payload,
    };
    Ok(serde_json::to_vec(&payload_in)?)
}

fn decode_challenge(out: &HttpOutV1) -> Option<String> {
    if out.body_b64.is_empty() {
        return None;
    }
    STANDARD.decode(&out.body_b64)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

#[test]
fn universal_ops_conformance() -> Result<()> {
    for spec in PROVIDERS {
        run_provider_checks(spec)?;
    }
    Ok(())
}

fn run_provider_checks(spec: &ProviderSpec) -> Result<()> {
    if spec.skip_universal_ops {
        eprintln!(
            "[provider-tests] skipping universal_ops_conformance for {:?}",
            spec.id
        );
        return Ok(());
    }
    let mut harness = ProviderHarness::new(spec)?;
    if spec.ingest_supported {
        let fixture = load_http_fixture(spec.fixture)?;
        let http_in = http_input_from_fixture(fixture);
        let ingest_bytes = serde_json::to_vec(&http_in)?;
        let ingest_out = harness.call("ingest_http", ingest_bytes)?;
        let http_out: HttpOutV1 = serde_json::from_slice(&ingest_out)?;
        assert_eq!(http_out.status, 200, "{:?} should support ingest", spec.id);
        assert!(
            !http_out.events.is_empty(),
            "{:?} ingest should emit events",
            spec.id
        );
    }
    if let Some(challenge_fixture) = spec.challenge_fixture {
        let challenge_input = http_input_from_fixture(load_http_fixture(challenge_fixture)?);
        let challenge_bytes = serde_json::to_vec(&challenge_input)?;
        let challenge_out = harness.call("ingest_http", challenge_bytes)?;
        let out: HttpOutV1 = serde_json::from_slice(&challenge_out)?;
        assert_eq!(out.status, 200, "challenge should return 200");
        if let Some(expected) = spec.challenge_response {
            let value = decode_challenge(&out).expect("challenge body");
            assert_eq!(value, expected, "challenge response mismatch");
        }
    }

    let message = build_envelope(spec.id);
    let plan_in = RenderPlanInV1 {
        message: message.clone(),
        metadata: HashMap::new(),
    };
    let plan_bytes = serde_json::to_vec(&plan_in)?;
    let plan_out_bytes = harness.call("render_plan", plan_bytes)?;
    let plan_response: RenderPlanResponse = serde_json::from_slice(&plan_out_bytes)?;
    assert!(plan_response.ok, "{:?} render_plan failed", spec.id);
    assert!(
        plan_response.plan.is_some(),
        "{:?} render_plan missing plan",
        spec.id
    );

    let encode_in = EncodeInV1 {
        message: message.clone(),
        plan: plan_in,
    };
    let encode_bytes = serde_json::to_vec(&encode_in)?;
    let encode_out = harness.call("encode", encode_bytes)?;
    let encode_response: EncodeResponse = serde_json::from_slice(&encode_out)?;
    assert!(encode_response.ok, "{:?} encode failed", spec.id);
    assert!(
        encode_response.payload.is_some(),
        "{:?} encode missing payload",
        spec.id
    );

    let send_bytes = send_payload_in(spec)?;
    let send_out = harness.call("send_payload", send_bytes)?;
    let send_result: SendPayloadResultV1 = serde_json::from_slice(&send_out)?;
    assert!(
        !send_result.retryable,
        "{:?} send_payload should not retry",
        spec.id
    );
    Ok(())
}
