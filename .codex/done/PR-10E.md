# PR-10E.md (greentic-messaging-providers)
# Title: Microsoft Graph Teams provider-core pack + send op + schema

## Goal
Implement Teams messaging via Microsoft Graph as provider-core pack.

## Deliverables
- Schema with:
  - tenant_id, client_id, auth mode (refresh token ref / client secret ref), team_id/channel_id defaults
- Component implementing:
  - invoke("send") => Graph call
- Pack + extension metadata

## Tests
- Prefer contract tests with HTTP mock
- No live Graph in CI

## Acceptance criteria
- provider-core contract satisfied; pack ready for deployment.
