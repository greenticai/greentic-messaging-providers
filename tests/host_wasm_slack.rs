use std::cell::{Cell, RefCell};
use std::path::Path;

use wasmtime::{
    component::{Component, Linker},
    Config, Engine, Store,
};

// Generate bindings for the slack world, mapping imported interfaces to our host.
wasmtime::component::bindgen!({
    path: "components/slack/wit/slack",
    world: "slack",
    with: {
        "greentic:http/http-client@1.0.0": HttpHost,
        "greentic:secrets-store/secrets-store@1.0.0": SecretsHost,
        "greentic:state/state-store@1.0.0": StateHost,
        "greentic:telemetry/logger-api@1.0.0": TelemetryHost,
    },
});

#[derive(Clone)]
struct HostState {
    secret: Option<Vec<u8>>,
    http_fail_first: bool,
    http_calls: Cell<u32>,
    telemetry_calls: Cell<u32>,
    http_options: RefCell<Option<bindings::greentic::http::http_client::RequestOptions>>,
}

struct HttpHost;
struct SecretsHost;
struct StateHost;
struct TelemetryHost;

impl bindings::greentic::http::http_client::Host for HostState {
    fn send(
        &mut self,
        _req: bindings::greentic::http::http_client::Request,
        options: Option<bindings::greentic::http::http_client::RequestOptions>,
        _ctx: Option<bindings::greentic::interfaces_types::types::TenantCtx>,
    ) -> Result<bindings::greentic::http::http_client::Response, bindings::greentic::http::http_client::HostError> {
        let call = self.http_calls.get();
        self.http_calls.set(call + 1);
        self.http_options.replace(options);
        if self.http_fail_first && call == 0 {
            return Err(bindings::greentic::http::http_client::HostError {
                code: "timeout".into(),
                message: "simulated timeout".into(),
            });
        }
        Ok(bindings::greentic::http::http_client::Response {
            status: 200,
            headers: vec![],
            body: None,
        })
    }
}

impl bindings::greentic::secrets_store::secrets_store::Host for HostState {
    fn get(
        &mut self,
        key: String,
    ) -> Result<Option<Vec<u8>>, bindings::greentic::secrets_store::secrets_store::SecretsError> {
        if self.secret.is_some() {
            return Ok(self.secret.clone());
        }
        let _ = key;
        Ok(None)
    }
}

impl bindings::greentic::state::state_store::Host for HostState {
    fn read(
        &mut self,
        _key: bindings::greentic::interfaces_types::types::StateKey,
        _ctx: Option<bindings::greentic::interfaces_types::types::TenantCtx>,
    ) -> Result<Vec<u8>, bindings::greentic::state::state_store::HostError> {
        Err(bindings::greentic::state::state_store::HostError {
            code: "unimplemented".into(),
            message: "state not available in tests".into(),
        })
    }

    fn write(
        &mut self,
        _key: bindings::greentic::interfaces_types::types::StateKey,
        _bytes: Vec<u8>,
        _ctx: Option<bindings::greentic::interfaces_types::types::TenantCtx>,
    ) -> Result<bindings::greentic::state::state_store::OpAck, bindings::greentic::state::state_store::HostError> {
        Err(bindings::greentic::state::state_store::HostError {
            code: "unimplemented".into(),
            message: "state not available in tests".into(),
        })
    }

    fn delete(
        &mut self,
        _key: bindings::greentic::interfaces_types::types::StateKey,
        _ctx: Option<bindings::greentic::interfaces_types::types::TenantCtx>,
    ) -> Result<bindings::greentic::state::state_store::OpAck, bindings::greentic::state::state_store::HostError> {
        Err(bindings::greentic::state::state_store::HostError {
            code: "unimplemented".into(),
            message: "state not available in tests".into(),
        })
    }
}

impl bindings::greentic::telemetry::logger_api::Host for HostState {
    fn log(
        &mut self,
        _span: bindings::greentic::interfaces_types::types::SpanContext,
        _fields: Vec<(String, String)>,
        _ctx: Option<bindings::greentic::interfaces_types::types::TenantCtx>,
    ) -> Result<bindings::greentic::telemetry::logger_api::OpAck, bindings::greentic::telemetry::logger_api::HostError> {
        let call = self.telemetry_calls.get();
        self.telemetry_calls.set(call + 1);
        Ok(bindings::greentic::telemetry::logger_api::OpAck::Ok)
    }
}

fn make_engine() -> Engine {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.cache_config_load(false);
    Engine::new(&config).expect("engine")
}

fn load_slack_component(engine: &Engine) -> Component {
    let path = Path::new("packs/messaging-provider-bundle/components/slack.wasm");
    Component::from_file(engine, path).expect("component load")
}

fn instantiate_slack(engine: &Engine, state: HostState) -> (Store<HostState>, bindings::Slack) {
    let component = load_slack_component(engine);
    let mut linker = Linker::new(engine);
    bindings::add_to_linker(&mut linker, |s: &mut HostState| s).expect("linker");
    let mut store = Store::new(engine, state);
    let instance = bindings::Slack::instantiate(&mut store, &component, &linker).expect("instantiate");
    (store, instance)
}

#[test]
fn slack_injected_config_controls_retries() {
    let engine = make_engine();
    let state = HostState {
        secret: Some(b"TEST".to_vec()),
        http_fail_first: true,
        http_calls: Cell::new(0),
        telemetry_calls: Cell::new(0),
        http_options: RefCell::new(None),
    };
    let (mut store, slack) = instantiate_slack(&engine, state);

    slack
        .call_init_runtime_config(
            &mut store,
            r#"{"schema_version":1,"network":{"max_attempts":2},"telemetry":{"emit_enabled":true,"service_name":"slack-test"}}"#.into(),
        )
        .expect("init config");

    let resp = slack
        .call_send_message(&mut store, "C123".into(), "hello".into())
        .expect("send ok");
    assert!(resp.contains("\"channel\":\"C123\""));

    let calls = store.data().http_calls.get();
    assert_eq!(calls, 2, "should retry once before succeeding");
    let opts = store
        .data()
        .http_options
        .borrow()
        .clone()
        .expect("options");
    assert!(matches!(opts.proxy, bindings::greentic::http::http_client::ProxyMode::Inherit));
    assert!(matches!(opts.tls, bindings::greentic::http::http_client::TlsMode::Strict));
    assert_eq!(store.data().telemetry_calls.get(), 1, "should emit one telemetry log");
}

#[test]
fn slack_missing_secret_surfaces_structured_error() {
    let engine = make_engine();
    let state = HostState {
        secret: None,
        http_fail_first: false,
        http_calls: Cell::new(0),
        telemetry_calls: Cell::new(0),
        http_options: RefCell::new(None),
    };
    let (mut store, slack) = instantiate_slack(&engine, state);

    slack
        .call_init_runtime_config(&mut store, r#"{"schema_version":1}"#.into())
        .expect("init config");

    let err = slack
        .call_send_message(&mut store, "C123".into(), "hello".into())
        .expect_err("should fail without secret");
    let value: serde_json::Value = serde_json::from_str(&err).expect("json error");
    assert!(value.get("MissingSecret").is_some(), "expected MissingSecret");
    assert_eq!(value["MissingSecret"]["name"], "SLACK_BOT_TOKEN");
    assert_eq!(value["MissingSecret"]["scope"], "tenant");
}
