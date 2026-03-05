# Operator Deep Dive

## Repo Structure: greentic-operator

```
greentic-operator/
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── cli.rs                       # ALL CLI commands (very large file)
│   ├── demo/
│   │   ├── mod.rs
│   │   ├── runtime.rs               # demo_up(), ServiceTracker, process management
│   │   └── runner.rs                # DemoRunner (flow REPL)
│   ├── messaging_universal/
│   │   ├── egress.rs                # Outbound message pipeline
│   │   ├── ingress.rs               # Inbound webhook handling
│   │   └── ...
│   ├── domains/
│   │   └── mod.rs                   # Domain/pack discovery, plan_runs
│   ├── discovery.rs                 # Provider pack discovery
│   ├── providers.rs                 # Provider setup orchestration
│   ├── component_qa_ops.rs          # QA contract (qa-spec, apply-answers, i18n-keys)
│   ├── setup_input.rs               # Interactive setup input
│   ├── runner_exec.rs               # Embedded runner (greentic-runner-desktop)
│   ├── runner_integration.rs        # External runner shell-out
│   ├── runner_host.rs               # DemoRunnerHost (PackRuntime wrapper)
│   ├── http_ingress.rs              # HttpIngressServer (Axum)
│   ├── secrets_gate.rs              # Secrets manager resolution
│   ├── provider_config_envelope.rs  # Config persistence
│   └── ...
├── tests/
│   ├── demo_run.rs
│   ├── demo_build.rs
│   ├── demo_setup_all_providers.rs
│   ├── op02_offline_e2e.rs
│   ├── op02_fixture_registry.rs
│   └── ...
├── docs/
│   ├── demo-run.md
│   ├── domains/messaging.md
│   └── ...
└── ci/
    └── local_check.sh
```

## Key Concepts

### RunnerMode

| Mode | When | How |
|------|------|-----|
| `Exec` | No `--runner-binary` flag | In-process via `greentic-runner-desktop` |
| `Integration` | `--runner-binary <path>` | Shells out to external runner binary |

For **component ops** (`render_plan`, `encode`, `send_payload`), always uses `PackRuntime` regardless of mode.
For **flow ops** (`setup_default`, `ingest_http`), uses mode-dependent execution.

### DemoRunnerHost

Central execution engine. Created per demo session:
```rust
DemoRunnerHost::new(bundle, discovery, runner_binary, secrets_handle, verbose)
```

Key methods:
- `invoke_provider_op(domain, provider_type, op, input)` - flow-based execution
- `invoke_provider_component_op(domain, pack, provider_type, op, input)` - direct WASM component invocation

### HttpIngressServer

Axum HTTP server started during `demo start`:
- Listens on gateway host:port from config
- Routes: `POST/GET /{domain}/ingress/{provider}/{tenant}/{team?}`
- Calls `DemoRunnerHost.invoke_provider_op("ingest_http", ...)`
- Returns `HttpOutV1` response

---

## demo setup - Detailed Flow

```
DemoSetupArgs::run()
  ├── ensure_cbor_packs(bundle)
  ├── discovery::discover_with_options(bundle, cbor_only=true)
  ├── discovery::persist(bundle, tenant, discovery)
  ├── resolve_domains → [Messaging, Events, Secrets]
  └── for each domain:
      run_domain_command(DomainRunArgs)
        ├── discover_provider_packs_cbor_only
        ├── filter by discovered_providers, demo_provider_files, allowed_providers
        ├── build setup_answers (from --setup-input or interactive)
        ├── domains::plan_runs(domain, Setup, packs, filter)
        │   → Vec<PlannedRun> { flow_id: "setup_default", pack_path, ... }
        ├── resolve_demo_runner_binary
        └── run_plan(planned_runs)
            └── for each pack:
                run_provider_setup_flow
                  ├── secrets_setup::ensure_pack_secrets
                  ├── collect_setup_answers
                  ├── component_qa_ops::apply_answers_via_component_qa (if supported)
                  │   ├── invoke "qa-spec" op
                  │   ├── invoke "i18n-keys" op
                  │   └── invoke "apply-answers" op
                  ├── build_input → JSON payload
                  ├── runner_integration::run_flow("setup_default", input)
                  ├── write_run_output
                  └── write_provider_config_envelope
```

## demo start - Detailed Flow

```
DemoUpArgs::run_start()
  └── run_with_shutdown(ctx)
      ├── select_bundle_run_targets → [(tenant, team)]
      ├── ensure_cbor_packs
      ├── discovery::discover_with_options
      ├── load OperatorConfig + DemoConfig
      ├── determine messaging_enabled, events_enabled
      ├── resolve NatsMode (Off|On|External)
      ├── (if --setup-input) run_demo_up_setup → run_domain_command(Setup)
      └── for each (tenant, team):
          demo::demo_up(...)
            ├── start cloudflared (if configured) → get public URL
            ├── start NATS (if NatsMode::On)
            ├── start messaging (if enabled && NATS)
            │   └── spawn_embedded_messaging → re-launch as "dev embedded"
            ├── start event components (if enabled)
            └── (if NOT NatsMode::On) → embedded runner mode
      ├── HttpIngressServer::start(addr, port, runner_host)
      └── await Ctrl+C → shutdown
```

## demo send - 3-Phase Pipeline

```
DemoSendArgs::run()
  ├── resolve_demo_provider_pack(bundle, tenant, team, "telegram")
  ├── primary_provider_type → "messaging.telegram.bot"
  ├── secrets_gate::resolve_secrets_manager
  ├── DemoRunnerHost::new(...)
  ├── build_demo_send_message → ChannelMessageEnvelope
  │
  ├── PHASE 1: render_plan
  │   └── invoke_provider_component_op("render_plan", envelope)
  │       → RenderPlanOutV1 { plan: { plan_json } }
  │
  ├── PHASE 2: encode
  │   └── invoke_provider_component_op("encode", envelope + plan)
  │       → EncodeOutV1 { payload: ProviderPayloadV1 { body_b64, metadata_json } }
  │
  └── PHASE 3: send_payload
      └── invoke_provider_component_op("send_payload", payload)
          → Provider makes actual HTTP call (e.g. Telegram Bot API)
```

## Ingress Flow (Webhook Receive)

```
HTTP POST → HttpIngressServer
  ├── parse: domain, provider, tenant, team from URL path
  ├── build_ingress_request(provider, method, headers, query, body)
  ├── DemoRunnerHost.invoke_provider_op("ingest_http", request)
  │   └── PackRuntime invokes ingest_http flow/op
  │       → HttpOutV1 { status, headers, body, events: [ChannelMessageEnvelope] }
  ├── (if end-to-end) for each event:
  │   egress::run_end_to_end
  │     ├── run_app_flow (process message through app pack)
  │     ├── render_plan
  │     ├── encode
  │     └── send_payload (with retry)
  └── return HTTP response
```

## Provider Pack Requirements for Operator

For a provider to work fully in the operator:

### Must have (in pack.yaml)
- `greentic.provider-extension.v1` with `ops` listing ALL supported ops
- `messaging.provider_flow_hints` with flow ID mappings

### Must implement (component ops)
| Op | When called |
|----|------------|
| `describe` | Pack discovery |
| `render_plan` | demo send phase 1 |
| `encode` | demo send phase 2 |
| `send_payload` | demo send phase 3 |
| `ingest_http` | Webhook ingress |
| `validate_config` | Setup validation |

### Must have (flows)
| Flow | When called |
|------|------------|
| `setup_default` | demo setup |
| `requirements` | demo send --print-required-args |

### Optional (flows)
| Flow | When called |
|------|------------|
| `verify_webhooks` | demo setup --verify-webhooks |
| `diagnostics` | demo diagnostics |
| `sync_subscriptions` | Subscription management (Teams only) |

### Optional (component ops for greentic-qa integration)
| Op | When called |
|----|------------|
| `qa-spec` | component_qa_ops during setup |
| `apply-answers` | component_qa_ops during setup |
| `i18n-keys` | component_qa_ops during setup |
