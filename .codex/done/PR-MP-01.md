PR-MP-01 — Add greentic-messaging-renderer crate inside greentic-messaging-providers

Repo: greentic-messaging-providers
Goal: Introduce the canonical rendering API + NO-OP Adaptive Card handling + tests (the “truth” lives here).

Deliverables

New crate: crates/greentic-messaging-renderer/

Minimal plan model + API:

RenderPlan

RenderItem (Text, AdaptiveCard, …)

RenderContext { target: Option<String> }

CardRenderer trait

NoopCardRenderer

render_plan_from_envelope(...) (or equivalent)

Tests that guarantee NO mutation:

v1.3 card unchanged

v1.4 card unchanged

mixed content unchanged

File list

crates/greentic-messaging-renderer/Cargo.toml

crates/greentic-messaging-renderer/src/lib.rs

crates/greentic-messaging-renderer/src/plan.rs

crates/greentic-messaging-renderer/src/errors.rs

crates/greentic-messaging-renderer/tests/noop_passthrough.rs

workspace Cargo.toml add member + deps

Key design constraints (enforced in this PR)

Renderer is provider-agnostic: it produces a plan, it does not “encode for Webex”.

Adaptive Card logic is pass-through only.

No downsampling; no version rewriting; no host-config transforms.

Test commands

cargo test -p greentic-messaging-renderer

Acceptance criteria

Renderer crate compiles and tests pass.

Renderer API is stable enough for operator to consume.