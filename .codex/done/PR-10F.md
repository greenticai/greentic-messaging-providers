# PR-10F.md (greentic-messaging-providers)
# Title: Email provider-core pack (SMTP or Graph) + schema + send op

## Goal
Implement email send via provider-core pack.

## Deliverables
- Schema for SMTP:
  - host, port, username, password_ref (x-secret), from_address
  - tls mode
- Component implementing invoke("send") mapping to email send request
- Pack + extension metadata

## Tests
- Local mock SMTP server or compile-only + dummy E2E

## Acceptance criteria
- Pack self-describing; send op implemented.
