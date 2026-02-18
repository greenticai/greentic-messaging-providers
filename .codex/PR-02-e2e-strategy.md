# PR-02: E2E testing strategy (fast mocks + nightly real)

## Goals
- Add nightly-real and fast-mock E2E strategy.

## Implementation Steps
1) Fast tests:
   - run with mocked host refs (http/secrets/state) and fixture components
   - assert canonical CBOR + deterministic keys

2) Nightly tests:
   - gated behind feature/env vars
   - hit real provider sandboxes where available (Slack/Telegram/etc.)
   - only validate connectivity + minimal message send/receive, not secrets exfiltration

3) Document how to run both.

## Acceptance Criteria
- Fast E2E runs in CI quickly.
- Nightly E2E can be enabled via env vars and is documented.


