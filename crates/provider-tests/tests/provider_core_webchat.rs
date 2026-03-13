use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use greentic_interfaces_wasmtime::host_helpers::v1::{
    HostFns, add_all_v1_to_linker, secrets_store, state_store,
};
use greentic_types::provider::PROVIDER_EXTENSION_ID;
use provider_common::component_v0_6::{DescribePayload, canonical_cbor_bytes, decode_cbor};
use serde_json::{Value, json};
use wasmtime::component::{Component, ComponentExportIndex, Linker, ResourceTable, TypedFunc};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../../components/messaging-provider-webchat/wit/messaging-provider-webchat",
        world: "component-v0-v6-v0",
    });
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn candidate_artifacts() -> Vec<PathBuf> {
    let root = workspace_root();
    vec![
        root.join("target/components/messaging-provider-webchat.wasm"),
        root.join("target/wasm32-wasip2/release/messaging_provider_webchat.wasm"),
        root.join("target/wasm32-wasip2/wasm32-wasip2/release/messaging_provider_webchat.wasm"),
        root.join("components/messaging-provider-webchat/target/wasm32-wasip2/release/messaging_provider_webchat.wasm"),
        root.join("packs/messaging-webchat/components/messaging-provider-webchat.wasm"),
    ]
}

fn ensure_component_artifact() -> Result<PathBuf> {
    for path in candidate_artifacts() {
        if path.exists() {
            return Ok(path);
        }
    }

    let status = Command::new("cargo")
        .args([
            "component",
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--package",
            "messaging-provider-webchat",
        ])
        .current_dir(workspace_root())
        .status()
        .context("running cargo component build for messaging-provider-webchat")?;
    if !status.success() {
        return Err(anyhow::anyhow!(
            "cargo component build failed with status {status}"
        ));
    }

    for path in candidate_artifacts() {
        if path.exists() {
            let target = workspace_root().join("target/components/messaging-provider-webchat.wasm");
            if !target.exists() {
                if let Some(dir) = target.parent() {
                    let _ = fs::create_dir_all(dir);
                }
                let _ = fs::copy(&path, &target);
            }
            return Ok(path);
        }
    }

    Err(anyhow::anyhow!(
        "component artifact missing after cargo component build"
    ))
}

fn new_engine() -> Engine {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.cache(None);
    Engine::new(&config).expect("engine")
}

#[derive(Default)]
struct HostState {
    table: ResourceTable,
    wasi_ctx: WasiCtx,
    secrets: HashMap<String, Vec<u8>>,
    state: HashMap<String, Vec<u8>>,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.table,
        }
    }
}

impl HostState {
    fn new() -> Self {
        Self {
            table: ResourceTable::new(),
            wasi_ctx: WasiCtxBuilder::new().inherit_stdio().build(),
            secrets: HashMap::new(),
            state: HashMap::new(),
        }
    }
}

impl secrets_store::SecretsStoreHostV1_1 for HostState {
    fn get(&mut self, key: String) -> Result<Option<Vec<u8>>, secrets_store::SecretsErrorV1_1> {
        Ok(self.secrets.get(&key).cloned())
    }

    fn put(&mut self, key: String, value: Vec<u8>) {
        self.secrets.insert(key, value);
    }
}

impl state_store::StateStoreHost for HostState {
    fn read(
        &mut self,
        key: state_store::StateKey,
        _ctx: Option<state_store::TenantCtx>,
    ) -> Result<Vec<u8>, state_store::StateStoreError> {
        self.state
            .get(&key)
            .cloned()
            .ok_or_else(|| state_store::StateStoreError {
                code: "not_found".into(),
                message: format!("state key not found: {key}"),
            })
    }

    fn write(
        &mut self,
        key: state_store::StateKey,
        bytes: Vec<u8>,
        _ctx: Option<state_store::TenantCtx>,
    ) -> Result<state_store::OpAck, state_store::StateStoreError> {
        self.state.insert(key, bytes);
        Ok(state_store::OpAck::Ok)
    }

    fn delete(
        &mut self,
        key: state_store::StateKey,
        _ctx: Option<state_store::TenantCtx>,
    ) -> Result<state_store::OpAck, state_store::StateStoreError> {
        self.state.remove(&key);
        Ok(state_store::OpAck::Ok)
    }
}

fn add_wasi_to_linker(linker: &mut Linker<HostState>) {
    wasmtime_wasi::p2::add_to_linker_sync(linker).expect("add wasi");
}

fn add_greentic_hosts(linker: &mut Linker<HostState>) {
    add_all_v1_to_linker(
        linker,
        HostFns {
            secrets_store_v1_1: Some(|state| state as &mut dyn secrets_store::SecretsStoreHostV1_1),
            state_store: Some(|state| state as &mut dyn state_store::StateStoreHost),
            ..Default::default()
        },
    )
    .expect("add greentic hosts");
}

#[test]
fn builds_webchat_component() -> Result<()> {
    let path = ensure_component_artifact()?;
    assert!(
        path.exists(),
        "component artifact should exist at {:?}",
        path
    );
    Ok(())
}

#[test]
fn pack_has_extension_and_schema() -> Result<()> {
    let pack_dir = workspace_root().join("packs/messaging-webchat");
    let manifest_path = pack_dir.join("pack.manifest.json");
    let manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).context("reading pack.manifest.json")?)
            .context("parsing pack.manifest.json")?;

    let provider_ext = manifest
        .get("extensions")
        .and_then(|ext| ext.get(PROVIDER_EXTENSION_ID))
        .expect("pack should include provider extension");
    assert_eq!(
        provider_ext
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
        PROVIDER_EXTENSION_ID
    );

    let providers = provider_ext
        .get("inline")
        .and_then(|inline| inline.get("providers"))
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(providers.len(), 1, "expected one provider entry");
    let provider = providers.first().expect("provider entry");
    assert_eq!(
        provider.get("provider_type").and_then(|v| v.as_str()),
        Some("messaging.webchat")
    );
    assert_eq!(
        provider
            .get("runtime")
            .and_then(|r| r.get("world"))
            .and_then(|v| v.as_str()),
        Some("greentic:provider/schema-core@1.0.0")
    );

    let schema_ref = provider
        .get("config_schema_ref")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert_eq!(
        schema_ref,
        "schemas/messaging/webchat/public.config.schema.json"
    );
    assert!(
        pack_dir.join(schema_ref).exists(),
        "pack schema should exist"
    );
    assert!(
        workspace_root()
            .join("schemas/messaging/webchat/public.config.schema.json")
            .exists(),
        "workspace schema should exist"
    );
    Ok(())
}

#[test]
fn invoke_send_and_ingest_smoke_test() -> Result<()> {
    let component_path = ensure_component_artifact()?;
    let engine = new_engine();
    let component = Component::from_file(&engine, &component_path)
        .map_err(|err| anyhow::anyhow!("loading component: {err}"))?;
    let mut linker = Linker::new(&engine);
    add_wasi_to_linker(&mut linker);
    add_greentic_hosts(&mut linker);

    let mut describe_store = Store::new(&engine, HostState::new());
    let instance = linker
        .instantiate(&mut describe_store, &component)
        .map_err(|err| anyhow::anyhow!("instantiate for describe: {err}"))?;

    let api_index: ComponentExportIndex = instance
        .get_export_index(
            &mut describe_store,
            None,
            "greentic:component/descriptor@0.6.0",
        )
        .context("get descriptor export index")?;

    let describe_index = instance
        .get_export_index(&mut describe_store, Some(&api_index), "describe")
        .context("get describe export index")?;
    let describe: TypedFunc<(), (Vec<u8>,)> = instance
        .get_typed_func(&mut describe_store, describe_index)
        .map_err(|err| anyhow::anyhow!("get describe func: {err}"))?;
    let (described,) = describe
        .call(&mut describe_store, ())
        .map_err(|err| anyhow::anyhow!("call describe: {err}"))?;
    let described: DescribePayload = decode_cbor(&described).map_err(anyhow::Error::msg)?;
    assert_eq!(described.provider, "messaging-provider-webchat");
    assert!(described.operations.iter().any(|op| op.name == "run"));
    assert!(described.operations.iter().any(|op| op.name == "send"));

    drop(describe_store);
    let mut store = Store::new(&engine, HostState::new());
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|err| anyhow::anyhow!("instantiate for invoke: {err}"))?;

    let api_index: ComponentExportIndex = instance
        .get_export_index(&mut store, None, "greentic:component/runtime@0.6.0")
        .context("get runtime export index for invoke")?;
    let invoke_index = instance
        .get_export_index(&mut store, Some(&api_index), "invoke")
        .context("get invoke export index")?;
    let invoke: TypedFunc<(String, Vec<u8>), (Vec<u8>,)> = instance
        .get_typed_func(&mut store, invoke_index)
        .map_err(|err| anyhow::anyhow!("get invoke func: {err}"))?;

    let input = json!({
        "route": "chat:abc",
        "text": "hello webchat",
        "mode": "local_queue",
        "config": {
            "route": "chat:abc",
            "public_base_url": "https://example.invalid"
        }
    });
    let input_bytes = canonical_cbor_bytes(&input);
    let (resp,) = invoke
        .call(&mut store, ("send".to_string(), input_bytes))
        .map_err(|err| anyhow::anyhow!("call invoke send: {err}"))?;
    let resp_json: Value = decode_cbor(&resp).map_err(anyhow::Error::msg)?;
    assert_eq!(resp_json.get("status"), Some(&Value::String("sent".into())));
    assert_eq!(
        resp_json.get("provider_type"),
        Some(&Value::String("messaging.webchat".into()))
    );

    let _ = invoke;
    let _ = instance;
    drop(store);

    let mut store = Store::new(&engine, HostState::new());
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|err| anyhow::anyhow!("instantiate for ingest: {err}"))?;
    let api_index: ComponentExportIndex = instance
        .get_export_index(&mut store, None, "greentic:component/runtime@0.6.0")
        .context("get runtime export index for ingest")?;
    let invoke_index = instance
        .get_export_index(&mut store, Some(&api_index), "invoke")
        .context("get invoke export index for ingest")?;
    let invoke: TypedFunc<(String, Vec<u8>), (Vec<u8>,)> = instance
        .get_typed_func(&mut store, invoke_index)
        .map_err(|err| anyhow::anyhow!("get invoke func for ingest: {err}"))?;

    let ingest_input = json!({"user_id":"u1","text":"hi"});
    let (ingest_resp,) = invoke
        .call(
            &mut store,
            ("ingest".to_string(), canonical_cbor_bytes(&ingest_input)),
        )
        .map_err(|err| anyhow::anyhow!("call invoke ingest: {err}"))?;
    let ingest_json: Value = decode_cbor(&ingest_resp).map_err(anyhow::Error::msg)?;
    assert_eq!(ingest_json.get("ok"), Some(&Value::Bool(true)));

    Ok(())
}
