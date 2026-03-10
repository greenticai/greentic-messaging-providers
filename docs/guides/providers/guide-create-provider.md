# Creating a New Channel Provider

Step-by-step guide for building a new messaging provider component for `greentic-operator`. Uses the **dummy** provider as a minimal reference.

## Overview

A provider is a WASM component (`wasm32-wasip2`) that bridges an external messaging service into Greentic. It's packaged as a `.gtpack` and deployed into the operator's bundle.

What you'll create:

```
greentic-messaging-providers/
├── components/messaging-provider-myapp/    # WASM component (Rust)
└── packs/messaging-myapp/                  # Pack (flows + assets + built gtpack)
```

## Prerequisites

```bash
# Rust 1.91+ with wasm32-wasip2 target
rustup default 1.91.0
rustup target add wasm32-wasip2

# Clone the providers repo
git clone git@github.com:greentic-ai/greentic-messaging-providers.git
cd greentic-messaging-providers
```

## 1. Create the Component

### Directory structure

```bash
mkdir -p components/messaging-provider-myapp/src
mkdir -p components/messaging-provider-myapp/wit/messaging-provider-myapp/deps/provider-schema-core
```

### Cargo.toml

```bash
cat > components/messaging-provider-myapp/Cargo.toml << 'EOF'
[package]
name = "messaging-provider-myapp"
version.workspace = true
edition.workspace = true
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
greentic-types.workspace = true
wit-bindgen.workspace = true
base64.workspace = true
provider-common.workspace = true

[package.metadata.component.target]
path = "wit/messaging-provider-myapp"
world = "component-v0-v6-v0"

[package.metadata.component.target.dependencies]
"greentic:provider-schema-core" = { path = "wit/messaging-provider-myapp/deps/provider-schema-core" }
EOF
```

Add to workspace `Cargo.toml`:

```bash
# In the root Cargo.toml [workspace] members list, add:
# "components/messaging-provider-myapp"
```

### WIT interfaces

The WIT defines what your component exports. Copy from the dummy provider:

```bash
cp components/messaging-provider-dummy/wit/messaging-provider-dummy/world.wit \
   components/messaging-provider-myapp/wit/messaging-provider-myapp/world.wit

cp components/messaging-provider-dummy/wit/messaging-provider-dummy/interfaces.wit \
   components/messaging-provider-myapp/wit/messaging-provider-myapp/interfaces.wit

cp -r components/messaging-provider-dummy/wit/messaging-provider-dummy/deps/provider-schema-core \
   components/messaging-provider-myapp/wit/messaging-provider-myapp/deps/
```

The WIT world (`world.wit`):

```wit
package greentic:component@0.6.1;

use greentic:provider-schema-core/schema-core-api@1.0.0;

world component-v0-v6-v0 {
  export descriptor;      // Component metadata + schemas
  export runtime;         // invoke(op, cbor) dispatch
  export qa;              // Setup/upgrade/remove forms
  export component-i18n;  // Localization keys
  export schema-core-api; // JSON-based backward-compat API
}
```

The interfaces (`interfaces.wit`):

```wit
interface descriptor {
  describe: func() -> list<u8>;   // Returns DescribePayload as CBOR
}

interface runtime {
  invoke: func(op: string, input-cbor: list<u8>) -> list<u8>;
}

interface qa {
  enum mode { default, setup, upgrade, remove }
  qa-spec: func(mode: mode) -> list<u8>;
  apply-answers: func(mode: mode, answers-cbor: list<u8>) -> list<u8>;
}

interface component-i18n {
  i18n-keys: func() -> list<string>;
  i18n-bundle: func(locale: string) -> list<u8>;
}
```

If your provider needs HTTP calls or secrets, also copy the `http` and `secrets-store` WIT deps from a provider that uses them (e.g. `messaging-provider-telegram`).

### lib.rs (minimal)

```bash
cat > components/messaging-provider-myapp/src/lib.rs << 'RUST'
use provider_common::component_v0_6::{
    DescribePayload, OperationDescriptor, SchemaIr, canonical_cbor_bytes, decode_cbor, schema_hash,
};
use provider_common::helpers::{existing_config_from_answers, i18n, string_or_default};
use provider_common::qa_helpers::ApplyAnswersResult;
use serde::{Deserialize, Serialize};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-myapp",
        world: "component-v0-v6-v0",
        generate_all
    });
}

const PROVIDER_ID: &str = "messaging-provider-myapp";
const WORLD_ID: &str = "component-v0-v6-v0";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProviderConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    webhook_url: String,
}

// ---------------------------------------------------------------------------
// I18n
// ---------------------------------------------------------------------------

const I18N_KEYS: &[&str] = &[
    "myapp.op.run.title",
    "myapp.op.run.description",
    "myapp.schema.input.title",
    "myapp.schema.config.title",
    "myapp.schema.config.api_key.title",
    "myapp.schema.config.webhook_url.title",
    "myapp.schema.output.title",
    "myapp.qa.setup.title",
];

const I18N_PAIRS: &[(&str, &str)] = &[
    ("myapp.op.run.title", "Send message"),
    ("myapp.op.run.description", "Send a message via MyApp"),
    ("myapp.schema.input.title", "MyApp input"),
    ("myapp.schema.config.title", "MyApp config"),
    ("myapp.schema.config.api_key.title", "API key"),
    ("myapp.schema.config.webhook_url.title", "Webhook URL"),
    ("myapp.schema.output.title", "MyApp output"),
    ("myapp.qa.setup.title", "Setup"),
];

// ---------------------------------------------------------------------------
// QA questions (for WASM qa-spec fallback)
// ---------------------------------------------------------------------------

const SETUP_QUESTIONS: &[(&str, bool)] = &[
    ("api_key", true),       // (field_name, is_secret)
    ("webhook_url", false),
];
const DEFAULT_KEYS: &[&str] = &["api_key", "webhook_url"];

// ---------------------------------------------------------------------------
// Describe
// ---------------------------------------------------------------------------

fn build_describe_payload() -> DescribePayload {
    DescribePayload {
        provider: PROVIDER_ID.to_string(),
        world: WORLD_ID.to_string(),
        operations: vec![OperationDescriptor {
            id: "run".to_string(),
            title: i18n("myapp.op.run.title"),
            description: i18n("myapp.op.run.description"),
        }],
        input_schema: SchemaIr::Object {
            title: i18n("myapp.schema.input.title"),
            description: i18n("myapp.schema.input.title"),
            properties: vec![],
            required: vec![],
            additional_properties: true,
        },
        output_schema: SchemaIr::Object {
            title: i18n("myapp.schema.output.title"),
            description: i18n("myapp.schema.output.title"),
            properties: vec![],
            required: vec![],
            additional_properties: true,
        },
        config_schema: SchemaIr::Object {
            title: i18n("myapp.schema.config.title"),
            description: i18n("myapp.schema.config.title"),
            properties: vec![
                ("api_key".into(), true, SchemaIr::String {
                    title: i18n("myapp.schema.config.api_key.title"),
                    description: i18n("myapp.schema.config.api_key.title"),
                    format: None,
                }),
                ("webhook_url".into(), true, SchemaIr::String {
                    title: i18n("myapp.schema.config.webhook_url.title"),
                    description: i18n("myapp.schema.config.webhook_url.title"),
                    format: Some("uri".into()),
                }),
            ],
            required: vec!["api_key".into()],
            additional_properties: false,
        },
        redactions: vec![],
        schema_hash: String::new(), // computed below
    }
}

// ---------------------------------------------------------------------------
// QA spec
// ---------------------------------------------------------------------------

fn build_qa_spec(mode: bindings::exports::greentic::component::qa::Mode) -> serde_json::Value {
    use bindings::exports::greentic::component::qa::Mode;
    let title = match mode {
        Mode::Default | Mode::Setup => "myapp.qa.setup.title",
        Mode::Upgrade => "myapp.qa.setup.title",
        Mode::Remove => "myapp.qa.setup.title",
    };
    serde_json::json!({
        "title": title,
        "questions": SETUP_QUESTIONS.iter().map(|(name, secret)| {
            serde_json::json!({
                "name": name,
                "title": format!("myapp.qa.setup.{name}"),
                "kind": "string",
                "required": true,
                "secret": secret,
            })
        }).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------
// Apply answers
// ---------------------------------------------------------------------------

fn apply_answers_impl(mode: &str, answers_cbor: Vec<u8>) -> Vec<u8> {
    let answers: serde_json::Value = decode_cbor(&answers_cbor).unwrap_or_default();
    let existing = existing_config_from_answers(&answers);
    let config = ProviderConfig {
        enabled: true,
        api_key: string_or_default(&answers, "api_key",
            existing.as_ref().and_then(|c: &serde_json::Value| c.get("api_key").and_then(|v| v.as_str())).unwrap_or("")),
        webhook_url: string_or_default(&answers, "webhook_url",
            existing.as_ref().and_then(|c: &serde_json::Value| c.get("webhook_url").and_then(|v| v.as_str())).unwrap_or("")),
    };
    let result = ApplyAnswersResult::ok(serde_json::to_value(&config).unwrap_or_default());
    canonical_cbor_bytes(&result)
}

// ---------------------------------------------------------------------------
// Op dispatch
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RunInput { message: String }

#[derive(Serialize)]
struct RunResult { ok: bool, message_id: Option<String>, error: Option<String> }

fn handle_run(input_cbor: &[u8]) -> Vec<u8> {
    let input: RunInput = match decode_cbor(input_cbor) {
        Ok(v) => v,
        Err(e) => return canonical_cbor_bytes(&RunResult { ok: false, message_id: None, error: Some(e) }),
    };
    // TODO: Replace with actual API call to your service
    let id = format!("myapp:{}", provider_common::component_v0_6::sha256_hex(input.message.as_bytes()));
    canonical_cbor_bytes(&RunResult { ok: true, message_id: Some(id), error: None })
}

// ---------------------------------------------------------------------------
// Trait implementations
// ---------------------------------------------------------------------------

struct Component;

impl bindings::exports::greentic::component::descriptor::Guest for Component {
    fn describe() -> Vec<u8> { canonical_cbor_bytes(&build_describe_payload()) }
}

impl bindings::exports::greentic::component::runtime::Guest for Component {
    fn invoke(op: String, input_cbor: Vec<u8>) -> Vec<u8> {
        match op.as_str() {
            "run" => handle_run(&input_cbor),
            _ => canonical_cbor_bytes(&RunResult { ok: false, message_id: None, error: Some(format!("unsupported op: {op}")) }),
        }
    }
}

impl bindings::exports::greentic::component::qa::Guest for Component {
    fn qa_spec(mode: bindings::exports::greentic::component::qa::Mode) -> Vec<u8> {
        canonical_cbor_bytes(&build_qa_spec(mode))
    }
    fn apply_answers(mode: bindings::exports::greentic::component::qa::Mode, answers_cbor: Vec<u8>) -> Vec<u8> {
        use bindings::exports::greentic::component::qa::Mode;
        let mode_str = match mode { Mode::Default => "default", Mode::Setup => "setup", Mode::Upgrade => "upgrade", Mode::Remove => "remove" };
        apply_answers_impl(mode_str, answers_cbor)
    }
}

impl bindings::exports::greentic::component::component_i18n::Guest for Component {
    fn i18n_keys() -> Vec<String> { provider_common::helpers::i18n_keys_from(I18N_KEYS) }
    fn i18n_bundle(locale: String) -> Vec<u8> { provider_common::helpers::i18n_bundle_from_pairs(locale, I18N_PAIRS) }
}

impl bindings::exports::greentic::provider_schema_core::schema_core_api::Guest for Component {
    fn describe() -> Vec<u8> { provider_common::helpers::schema_core_describe(&build_describe_payload()) }
    fn validate_config(_config_json: Vec<u8>) -> Vec<u8> { provider_common::helpers::schema_core_validate_config() }
    fn healthcheck() -> Vec<u8> { provider_common::helpers::schema_core_healthcheck() }
    fn invoke(op: String, input_json: Vec<u8>) -> Vec<u8> {
        if let Some(result) = provider_common::qa_invoke_bridge::dispatch_qa_ops_with_i18n(
            &op, &input_json, "myapp", SETUP_QUESTIONS, DEFAULT_KEYS, I18N_KEYS, I18N_PAIRS, apply_answers_impl,
        ) { return result; }
        match op.as_str() {
            "run" => {
                let input: RunInput = match serde_json::from_slice(&input_json) {
                    Ok(v) => v,
                    Err(e) => return serde_json::to_vec(&RunResult { ok: false, message_id: None, error: Some(e.to_string()) }).unwrap_or_default(),
                };
                let id = format!("myapp:{}", provider_common::component_v0_6::sha256_hex(input.message.as_bytes()));
                serde_json::to_vec(&RunResult { ok: true, message_id: Some(id), error: None }).unwrap_or_default()
            }
            _ => serde_json::to_vec(&RunResult { ok: false, message_id: None, error: Some(format!("unsupported op: {op}")) }).unwrap_or_default(),
        }
    }
}

bindings::export!(Component with_types_in bindings);
RUST
```

The key pattern:
- `runtime::invoke` uses CBOR (via `decode_cbor` / `canonical_cbor_bytes`)
- `schema_core_api::invoke` uses JSON (via `serde_json`)
- Both dispatch to the same logic
- `dispatch_qa_ops_with_i18n` handles `qa-spec`, `apply-answers`, `i18n-keys`, `i18n-bundle` ops automatically

## 2. Build the Component

```bash
cargo build --release --package messaging-provider-myapp --target wasm32-wasip2
```

Output: `target/wasm32-wasip2/release/messaging_provider_myapp.wasm`

Copy to the standard location:

```bash
mkdir -p target/components
mkdir -p target/components/messaging-provider-myapp
cp target/wasm32-wasip2/release/messaging_provider_myapp.wasm \
   target/components/messaging-provider-myapp/component.wasm
```

## 3. Create the Pack

### Directory structure

```bash
mkdir -p packs/messaging-myapp/flows
mkdir -p packs/messaging-myapp/components
mkdir -p packs/messaging-myapp/assets
```

### setup.yaml

```bash
cat > packs/messaging-myapp/assets/setup.yaml << 'EOF'
provider_id: myapp
version: 1
title: MyApp provider setup
questions:
  - name: api_key
    title: API key
    kind: string
    required: true
    secret: true
    help: "Your MyApp API key"
  - name: webhook_url
    title: Webhook URL
    kind: string
    required: false
    help: "Public URL for incoming webhooks"
    validate:
      regex: "^https://"
EOF
```

### pack.yaml

```bash
cat > packs/messaging-myapp/pack.yaml << 'EOF'
pack_id: messaging-myapp
version: 0.1.0
kind: application
publisher: Greentic
components:
  - id: messaging-provider-myapp
    version: 0.1.0
    world: greentic:provider/schema-core@1.0.0
    wasm: components/messaging-provider-myapp/component.wasm
flows:
  - id: setup_default
    file: flows/setup_default.ygtc
    entrypoints: [setup]
assets:
  - path: setup.yaml
extensions:
  greentic.provider-extension.v1:
    inline:
      providers:
        - provider_type: messaging.myapp
          capabilities: [messaging]
          ops: [run, qa-spec, apply-answers, i18n-keys]
          config_schema_ref: null
          runtime:
            component_ref: messaging-provider-myapp
            export: schema-core-api
            world: greentic:provider/schema-core@1.0.0
EOF
```

### Setup flow

```bash
cat > packs/messaging-myapp/flows/setup_default.ygtc << 'EOF'
id: setup_default
type: job
start: emit_questions
schema_version: 2
nodes:
  emit_questions:
    routing:
      - to: apply
    text:
      template: "Collecting MyApp setup inputs."
  apply:
    routing:
      - out: true
    text:
      template: "MyApp setup complete."
EOF
```

### Copy the WASM

```bash
mkdir -p packs/messaging-myapp/components/messaging-provider-myapp
cp target/components/messaging-provider-myapp/component.wasm \
   packs/messaging-myapp/components/messaging-provider-myapp/
```

### Build the gtpack

```bash
cd packs/messaging-myapp
greentic-pack build --in . --allow-pack-schema --offline
# Output: dist/messaging-myapp.gtpack
```

## 4. Deploy to Operator

```bash
# Copy gtpack to demo bundle
cp packs/messaging-myapp/dist/messaging-myapp.gtpack \
   demo-bundle/providers/messaging/

# Add tenant access
echo "messaging-myapp = public" >> demo-bundle/tenants/default/tenant.gmap
echo "messaging-myapp = public" >> demo-bundle/tenants/default/teams/default/team.gmap

# Seed secrets
tools/seed-secret/target/release/seed-secret \
  demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-myapp/api_key" "my-api-key-here"

# Start operator
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

### Verify

```bash
# Check provider is discovered
curl -s http://localhost:8080/api/onboard/providers | jq '.providers[] | select(.pack_id == "messaging-myapp")'

# Get setup form
curl -s -X POST http://localhost:8080/api/onboard/qa/spec \
  -H "Content-Type: application/json" \
  -d '{"provider_id": "messaging-myapp", "domain": "messaging", "tenant": "default", "answers": {}}' | jq .
```

## 5. Test the Component

### Unit test with messaging-tester

```bash
# Create values file
cat > /tmp/myapp-values.json << 'EOF'
{
  "config": {},
  "secrets": {"API_KEY": "test-key"},
  "to": {},
  "http": "mock",
  "state": {}
}
EOF

target/release/greentic-messaging-tester send \
  --provider messaging-myapp \
  --values /tmp/myapp-values.json \
  --text "Hello from MyApp"
```

### Snapshot tests

Add the test macro to your component:

```rust
#[cfg(test)]
mod tests {
    provider_common::standard_provider_tests! {
        describe_fn: super::build_describe_payload,
        qa_spec_fn: super::build_qa_spec,
        i18n_keys: super::I18N_KEYS,
        world_id: super::WORLD_ID,
        provider_id: super::PROVIDER_ID,
        mode_type: super::bindings::exports::greentic::component::qa::Mode,
        component_type: super::Component,
        qa_guest_path: super::bindings::exports::greentic::component::qa::Guest,
    }
}
```

```bash
cargo test --package messaging-provider-myapp
```

## Adding Real Functionality

The minimal component above only implements `run` (hash-based dummy). For a real provider, add these ops to `invoke()`:

| Op | Purpose | Input type | What to implement |
|----|---------|------------|-------------------|
| `render_plan` | Downsampling cards for your platform | `RenderPlanInV1` | Call `provider_common` renderer with your capability profile |
| `encode` | Convert envelope to your API format | `ChannelMessageEnvelope` | Serialize to your platform's message format |
| `send_payload` | Make the HTTP API call | `SendPayloadInV1` | HTTP POST to your service API |
| `ingest_http` | Parse incoming webhooks | `HttpInV1` | Parse webhook body, return `HttpOutV1` with events |

For HTTP calls, add the `http` WIT dep (copy from `messaging-provider-telegram/wit/*/deps/http/`).
For secrets access, add the `secrets-store` WIT dep.

See `components/messaging-provider-telegram/src/lib.rs` for a full-featured reference implementation.

## Checklist

- [ ] Component directory created with Cargo.toml, WIT, and lib.rs
- [ ] Added to workspace Cargo.toml members
- [ ] `cargo build --target wasm32-wasip2` compiles clean
- [ ] Pack directory created with pack.yaml, setup.yaml, flow, and WASM
- [ ] `greentic-pack build` produces `.gtpack`
- [ ] gtpack copied to `demo-bundle/providers/messaging/`
- [ ] Tenant .gmap updated
- [ ] Secrets seeded
- [ ] Operator discovers the provider (`/api/onboard/providers`)
- [ ] Setup form works (`/api/onboard/qa/spec`)
- [ ] `cargo test` passes
