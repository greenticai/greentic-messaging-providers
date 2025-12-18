use std::cell::{Cell, RefCell};
use std::path::Path;

use wasmtime::{
    component::{Component, Linker},
    Config, Engine, Store,
};

fn make_engine() -> Engine {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.cache_config_load(false);
    Engine::new(&config).expect("engine")
}

// --- Telegram ----------------------------------------------------------------
mod telegram {
    use super::*;

    wasmtime::component::bindgen!({
        path: "components/telegram/wit/telegram",
        world: "telegram",
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
            _key: String,
        ) -> Result<Option<Vec<u8>>, bindings::greentic::secrets_store::secrets_store::SecretsError> {
            Ok(self.secret.clone())
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

    fn component(engine: &Engine) -> Component {
        Component::from_file(
            engine,
            Path::new("packs/messaging-provider-bundle/components/telegram.wasm"),
        )
        .expect("component")
    }

    fn instantiate(engine: &Engine, state: HostState) -> (Store<HostState>, bindings::Telegram) {
        let comp = component(engine);
        let mut linker = Linker::new(engine);
        bindings::add_to_linker(&mut linker, |s: &mut HostState| s).expect("linker");
        let mut store = Store::new(engine, state);
        let inst = bindings::Telegram::instantiate(&mut store, &comp, &linker).expect("inst");
        (store, inst)
    }

    #[test]
    fn config_controls_retries() {
        let engine = super::make_engine();
        let state = HostState {
            secret: Some(b"token".to_vec()),
            http_fail_first: true,
            http_calls: Cell::new(0),
            telemetry_calls: Cell::new(0),
            http_options: RefCell::new(None),
        };
        let (mut store, tg) = instantiate(&engine, state);

        tg.call_init_runtime_config(
            &mut store,
            r#"{"schema_version":1,"network":{"max_attempts":2},"telemetry":{"emit_enabled":true,"service_name":"telegram-test"}}"#.into(),
        )
        .expect("init");

        let resp = tg
            .call_send_message(&mut store, "123".into(), "hi".into())
            .expect("send ok");
        assert!(resp.contains("\"chat_id\":\"123\""));
        assert_eq!(store.data().http_calls.get(), 2);
        let opts = store
            .data()
            .http_options
            .borrow()
            .clone()
            .expect("options");
        assert!(matches!(opts.proxy, bindings::greentic::http::http_client::ProxyMode::Inherit));
        assert!(matches!(opts.tls, bindings::greentic::http::http_client::TlsMode::Strict));
        assert_eq!(store.data().telemetry_calls.get(), 1);
    }

    #[test]
    fn missing_secret_is_structured() {
        let engine = super::make_engine();
        let state = HostState {
            secret: None,
            http_fail_first: false,
            http_calls: Cell::new(0),
            telemetry_calls: Cell::new(0),
            http_options: RefCell::new(None),
        };
        let (mut store, tg) = instantiate(&engine, state);
        tg.call_init_runtime_config(&mut store, r#"{"schema_version":1}"#.into())
            .expect("init");

        let err = tg
            .call_send_message(&mut store, "123".into(), "hi".into())
            .expect_err("should fail");
        let val: serde_json::Value = serde_json::from_str(&err).expect("json error");
        assert_eq!(val["MissingSecret"]["name"], "TELEGRAM_BOT_TOKEN");
        assert_eq!(val["MissingSecret"]["scope"], "tenant");
    }
}

// --- Webex -------------------------------------------------------------------
mod webex {
    use super::*;

    wasmtime::component::bindgen!({
        path: "components/webex/wit/webex",
        world: "webex",
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
            _key: String,
        ) -> Result<Option<Vec<u8>>, bindings::greentic::secrets_store::secrets_store::SecretsError> {
            Ok(self.secret.clone())
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

    fn component(engine: &Engine) -> Component {
        Component::from_file(
            engine,
            Path::new("packs/messaging-provider-bundle/components/webex.wasm"),
        )
        .expect("component")
    }

    fn instantiate(engine: &Engine, state: HostState) -> (Store<HostState>, bindings::Webex) {
        let comp = component(engine);
        let mut linker = Linker::new(engine);
        bindings::add_to_linker(&mut linker, |s: &mut HostState| s).expect("linker");
        let mut store = Store::new(engine, state);
        let inst = bindings::Webex::instantiate(&mut store, &comp, &linker).expect("inst");
        (store, inst)
    }

    #[test]
    fn config_controls_retries() {
        let engine = super::make_engine();
        let state = HostState {
            secret: Some(b"token".to_vec()),
            http_fail_first: true,
            http_calls: Cell::new(0),
            telemetry_calls: Cell::new(0),
            http_options: RefCell::new(None),
        };
        let (mut store, comp) = instantiate(&engine, state);

        comp.call_init_runtime_config(
            &mut store,
            r#"{"schema_version":1,"network":{"max_attempts":2},"telemetry":{"emit_enabled":true,"service_name":"webex-test"}}"#.into(),
        )
        .expect("init");

        let resp = comp
            .call_send_message(&mut store, "room1".into(), "hi".into())
            .expect("send ok");
        assert!(resp.contains("\"roomId\":\"room1\""));
        assert_eq!(store.data().http_calls.get(), 2);
        let opts = store
            .data()
            .http_options
            .borrow()
            .clone()
            .expect("options");
        assert!(matches!(opts.proxy, bindings::greentic::http::http_client::ProxyMode::Inherit));
        assert!(matches!(opts.tls, bindings::greentic::http::http_client::TlsMode::Strict));
        assert_eq!(store.data().telemetry_calls.get(), 1);
    }

    #[test]
    fn missing_secret_is_structured() {
        let engine = super::make_engine();
        let state = HostState {
            secret: None,
            http_fail_first: false,
            http_calls: Cell::new(0),
            telemetry_calls: Cell::new(0),
            http_options: RefCell::new(None),
        };
        let (mut store, comp) = instantiate(&engine, state);
        comp.call_init_runtime_config(&mut store, r#"{"schema_version":1}"#.into())
            .expect("init");

        let err = comp
            .call_send_message(&mut store, "room1".into(), "hi".into())
            .expect_err("should fail");
        let val: serde_json::Value = serde_json::from_str(&err).expect("json error");
        assert_eq!(val["MissingSecret"]["name"], "WEBEX_BOT_TOKEN");
    }
}

// --- WhatsApp ----------------------------------------------------------------
mod whatsapp {
    use super::*;

    wasmtime::component::bindgen!({
        path: "components/whatsapp/wit/whatsapp",
        world: "whatsapp",
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
            _key: String,
        ) -> Result<Option<Vec<u8>>, bindings::greentic::secrets_store::secrets_store::SecretsError> {
            Ok(self.secret.clone())
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

    fn component(engine: &Engine) -> Component {
        Component::from_file(
            engine,
            Path::new("packs/messaging-provider-bundle/components/whatsapp.wasm"),
        )
        .expect("component")
    }

    fn instantiate(engine: &Engine, state: HostState) -> (Store<HostState>, bindings::Whatsapp) {
        let comp = component(engine);
        let mut linker = Linker::new(engine);
        bindings::add_to_linker(&mut linker, |s: &mut HostState| s).expect("linker");
        let mut store = Store::new(engine, state);
        let inst = bindings::Whatsapp::instantiate(&mut store, &comp, &linker).expect("inst");
        (store, inst)
    }

    #[test]
    fn config_controls_retries() {
        let engine = super::make_engine();
        let state = HostState {
            secret: Some(b"token".to_vec()),
            http_fail_first: true,
            http_calls: Cell::new(0),
            telemetry_calls: Cell::new(0),
            http_options: RefCell::new(None),
        };
        let (mut store, comp) = instantiate(&engine, state);

        comp.call_init_runtime_config(
            &mut store,
            r#"{"schema_version":1,"network":{"max_attempts":2},"telemetry":{"emit_enabled":true,"service_name":"whatsapp-test"}}"#.into(),
        )
        .expect("init");

        let dest = r#"{"phone_number_id":"pn1","to":"+100"}"#.to_string();
        let resp = comp
            .call_send_message(&mut store, dest, "hi".into())
            .expect("send ok");
        assert!(resp.contains("\"messaging_product\":\"whatsapp\""));
        assert_eq!(store.data().http_calls.get(), 2);
        let opts = store
            .data()
            .http_options
            .borrow()
            .clone()
            .expect("options");
        assert!(matches!(opts.proxy, bindings::greentic::http::http_client::ProxyMode::Inherit));
        assert!(matches!(opts.tls, bindings::greentic::http::http_client::TlsMode::Strict));
        assert_eq!(store.data().telemetry_calls.get(), 1);
    }

    #[test]
    fn missing_secret_is_structured() {
        let engine = super::make_engine();
        let state = HostState {
            secret: None,
            http_fail_first: false,
            http_calls: Cell::new(0),
            telemetry_calls: Cell::new(0),
            http_options: RefCell::new(None),
        };
        let (mut store, comp) = instantiate(&engine, state);
        comp.call_init_runtime_config(&mut store, r#"{"schema_version":1}"#.into())
            .expect("init");

        let dest = r#"{"phone_number_id":"pn1","to":"+100"}"#.to_string();
        let err = comp
            .call_send_message(&mut store, dest, "hi".into())
            .expect_err("should fail");
        let val: serde_json::Value = serde_json::from_str(&err).expect("json error");
        assert_eq!(val["MissingSecret"]["name"], "WHATSAPP_TOKEN");
    }
}

// --- Teams -------------------------------------------------------------------
mod teams {
    use super::*;

    wasmtime::component::bindgen!({
        path: "components/teams/wit/teams",
        world: "teams",
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
            _key: String,
        ) -> Result<Option<Vec<u8>>, bindings::greentic::secrets_store::secrets_store::SecretsError> {
            Ok(self.secret.clone())
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

    fn component(engine: &Engine) -> Component {
        Component::from_file(
            engine,
            Path::new("packs/messaging-provider-bundle/components/teams.wasm"),
        )
        .expect("component")
    }

    fn instantiate(engine: &Engine, state: HostState) -> (Store<HostState>, bindings::Teams) {
        let comp = component(engine);
        let mut linker = Linker::new(engine);
        bindings::add_to_linker(&mut linker, |s: &mut HostState| s).expect("linker");
        let mut store = Store::new(engine, state);
        let inst = bindings::Teams::instantiate(&mut store, &comp, &linker).expect("inst");
        (store, inst)
    }

    #[test]
    fn config_controls_retries() {
        let engine = super::make_engine();
        let state = HostState {
            secret: Some(b"token".to_vec()),
            http_fail_first: true,
            http_calls: Cell::new(0),
            telemetry_calls: Cell::new(0),
            http_options: RefCell::new(None),
        };
        let (mut store, comp) = instantiate(&engine, state);

        comp.call_init_runtime_config(
            &mut store,
            r#"{"schema_version":1,"network":{"max_attempts":2},"telemetry":{"emit_enabled":true,"service_name":"teams-test"}}"#.into(),
        )
        .expect("init");

        let dest = r#"{"team_id":"t1","channel_id":"c1"}"#.to_string();
        let resp = comp
            .call_send_message(&mut store, dest, "hi".into())
            .expect("send ok");
        assert!(resp.contains("\"team_id\":\"t1\""));
        assert_eq!(store.data().http_calls.get(), 2);
        let opts = store
            .data()
            .http_options
            .borrow()
            .clone()
            .expect("options");
        assert!(matches!(opts.proxy, bindings::greentic::http::http_client::ProxyMode::Inherit));
        assert!(matches!(opts.tls, bindings::greentic::http::http_client::TlsMode::Strict));
        assert_eq!(store.data().telemetry_calls.get(), 1);
    }

    #[test]
    fn missing_secret_is_structured() {
        let engine = super::make_engine();
        let state = HostState {
            secret: None,
            http_fail_first: false,
            http_calls: Cell::new(0),
            telemetry_calls: Cell::new(0),
            http_options: RefCell::new(None),
        };
        let (mut store, comp) = instantiate(&engine, state);
        comp.call_init_runtime_config(&mut store, r#"{"schema_version":1}"#.into())
            .expect("init");

        let dest = r#"{"team_id":"t1","channel_id":"c1"}"#.to_string();
        let err = comp
            .call_send_message(&mut store, dest, "hi".into())
            .expect_err("should fail");
        let val: serde_json::Value = serde_json::from_str(&err).expect("json error");
        assert_eq!(val["MissingSecret"]["name"], "MS_GRAPH_TENANT_ID");
    }
}

// --- WebChat -----------------------------------------------------------------
mod webchat {
    use super::*;

    wasmtime::component::bindgen!({
        path: "components/webchat/wit/webchat",
        world: "webchat",
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
            _key: String,
        ) -> Result<Option<Vec<u8>>, bindings::greentic::secrets_store::secrets_store::SecretsError> {
            Ok(self.secret.clone())
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

    fn component(engine: &Engine) -> Component {
        Component::from_file(
            engine,
            Path::new("packs/messaging-provider-bundle/components/webchat.wasm"),
        )
        .expect("component")
    }

    fn instantiate(engine: &Engine, state: HostState) -> (Store<HostState>, bindings::Webchat) {
        let comp = component(engine);
        let mut linker = Linker::new(engine);
        bindings::add_to_linker(&mut linker, |s: &mut HostState| s).expect("linker");
        let mut store = Store::new(engine, state);
        let inst = bindings::Webchat::instantiate(&mut store, &comp, &linker).expect("inst");
        (store, inst)
    }

    #[test]
    fn config_controls_retries() {
        let engine = super::make_engine();
        let state = HostState {
            secret: None,
            http_fail_first: true,
            http_calls: Cell::new(0),
            telemetry_calls: Cell::new(0),
            http_options: RefCell::new(None),
        };
        let (mut store, comp) = instantiate(&engine, state);

        comp.call_init_runtime_config(
            &mut store,
            r#"{"schema_version":1,"network":{"max_attempts":2},"telemetry":{"emit_enabled":true,"service_name":"webchat-test"}}"#.into(),
        )
        .expect("init");

        let resp = comp
            .call_send_message(&mut store, "sess-1".into(), "hi".into())
            .expect("send ok");
        assert!(resp.contains("\"session_id\":\"sess-1\""));
        assert_eq!(store.data().http_calls.get(), 2);
        let opts = store
            .data()
            .http_options
            .borrow()
            .clone()
            .expect("options");
        assert!(matches!(opts.proxy, bindings::greentic::http::http_client::ProxyMode::Inherit));
        assert!(matches!(opts.tls, bindings::greentic::http::http_client::TlsMode::Strict));
        assert_eq!(store.data().telemetry_calls.get(), 1);
    }

    #[test]
    fn optional_secret_absent_is_ok() {
        let engine = super::make_engine();
        let state = HostState {
            secret: None,
            http_fail_first: false,
            http_calls: Cell::new(0),
            telemetry_calls: Cell::new(0),
            http_options: RefCell::new(None),
        };
        let (mut store, comp) = instantiate(&engine, state);
        comp.call_init_runtime_config(&mut store, r#"{"schema_version":1}"#.into())
            .expect("init");

        let resp = comp
            .call_send_message(&mut store, "sess-1".into(), "hi".into())
            .expect("send ok");
        assert!(resp.contains("\"session_id\":\"sess-1\""));
    }
}
