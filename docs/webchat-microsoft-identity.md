# WebChat "Sign in with Microsoft": authority and verified identity

Status: parts (a) (`oauth_microsoft_authority`) and (b) (verified identity,
below) are implemented. Nothing here was exercised against a real Entra tenant:
the verifier is covered by unit tests with locally generated RSA keys.

## What exists today

| Piece | Where | Behaviour |
|---|---|---|
| Button config | `gui_core::compose_oauth_providers`, `ops/oauth.rs::handle_auth_config` | Microsoft provider entry with `auth_url`/`token_url` on `login.microsoftonline.com/<authority>/oauth2/v2.0/...`. Was hard-coded to `common`. |
| Browser flow | `assets/webchat-gui/runtime-bootstrap.js`, `oauth.js` | Authorization code + PKCE in the browser; the code is exchanged through `POST /oauth/token-exchange`. |
| Session | `handleTokens` in `runtime-bootstrap.js` | Stores `access_token \|\| id_token` as `token_handle` and decodes `name`/`email`/`picture` from the id_token **without verifying it**, for display only. |
| Direct Line mint | `webchat-directline-core::http::handle_tokens` | Mints a token with `verified: true` only when the request carries `Authorization: Bearer` that verifies; otherwise anonymous. |
| Bearer flag | `runtime-bootstrap.js` (`greentic_bearer`) | Set only on the Greentic SSO fallback path. A Microsoft login never sets it, so `/token` is called with no `Authorization` header. |

Before part (b) the Microsoft button was only a UI gate: the Direct Line token
was anonymous and `extensions.caller` carried `user_verified: false`. The table
above describes that earlier state; part (b) below replaces the last two rows
for Microsoft.

## Why the existing verifier cannot be reused

`oidc::verify_access_token` is written for the Greentic issuer:

- ES256 only (`alg != "ES256"` is `UnsupportedAlg`; JWKS keys must be EC P-256).
  Entra signs with RS256.
- JWKS is fetched from `<issuer>/jwks.json`. Entra publishes its JWKS through
  OIDC discovery (`.well-known/openid-configuration` -> `jwks_uri`).
- Requires a `scope` claim containing `greentic.webchat`. Entra ID tokens carry
  no `scope`.
- `aud` must equal one string and `iss` must equal one configured value. For the
  `common` and `organizations` authorities Entra's `iss` is
  `https://login.microsoftonline.com/{tid}/v2.0` with the caller's own tenant
  id, so there is no single expected issuer.

## Part (a): configurable authority (implemented)

New optional setup answer `oauth_microsoft_authority`.

- Unset or blank = `common`; the composed URLs are byte-identical to before
  (pinned by a test).
- Accepts `common`, `organizations`, `consumers`, a tenant GUID, or a tenant
  domain. Charset is `[A-Za-z0-9.-]`, at most 128 characters, no leading,
  trailing or doubled dots, so the value can only ever be one path segment of
  the Microsoft URL.
- Invalid at setup: `apply-answers` returns `ok: false`
  (`oauth_microsoft_authority must be ...`).
- Invalid when read at run time (raw answers written by `greentic-setup`, which
  never runs `apply-answers`): the Microsoft button is omitted. It never falls
  back to `common`, because silently widening a pinned authority is the unsafe
  direction.
- The question is optional with no default, in `describe.rs` (shared by the
  `webchat`, `webchat-gui` and `3aigent-gui` providers) and in both GUI packs'
  `setup.yaml`. `greentic-setup --emit-answers` gains one optional,
  non-secret field; consumers that ignore unknown optional fields are unaffected.
- The Entra app registration must be created for the matching audience
  (single tenant, multi tenant, or personal accounts) or Entra refuses the login.

Not changed: `POST /oauth/token-exchange` still reads `token_url` from the
request body and attaches the stored `client_secret` for the named provider to
a POST to that URL. A caller can therefore point it at any HTTPS host. Pinning
`token_url` to the configured provider is a separate hardening change, and is
easy now that the authority is a single configured value.

## Part (b): the verified Microsoft identity

### Flow

1. The browser sends a random `nonce` on the authorize request (kept in session
   storage) and keeps the **id_token** from the token exchange.
2. `POST /token` carries `Authorization: Bearer <id_token>` plus
   `X-Greentic-Idp: microsoft` and `X-Greentic-Nonce: <nonce>`. The header pair
   is what selects the Microsoft path; without it the existing Greentic bearer
   path is untouched. An id_token that has expired signs the visitor out rather
   than minting an anonymous token behind a signed-in UI.
3. `webchat-directline-core::oidc_entra` verifies it (RS256 via the `rsa`
   crate, see below) and the Direct Line token is minted `verified: true` with
   `sub = ms:<tid>:<oid>` and the extra claims `idp_iss`, `tid`, `oid` and,
   when present, `email` (`preferred_username`, else `email`) and `name`.
4. `caller_block` publishes `tid`, `oid`, `email` and `name` in
   `extensions.caller` only for a token whose `idp_iss` is a
   `login.microsoftonline.com` issuer, so no other issuer's extra claims start
   appearing. The runner's typed readers ignore the extra keys; a flow reads them
   from the raw block.

### What is verified

Discovery `https://login.microsoftonline.com/<authority>/v2.0/.well-known/openid-configuration`
gives `jwks_uri`, which must be https on `login.microsoftonline.com`. Both
documents go through the same cache and refetch-on-unknown-`kid` cooldown as the
Greentic JWKS. Then: signature (`alg` must be RS256, `kid` matched), the token's
own `tid` is a GUID and `iss == https://login.microsoftonline.com/<tid>/v2.0`,
`aud == oauth_microsoft_client_id`, `exp`/`nbf` with 60 s leeway, `nonce` equal
to the browser's, and an `oid`.

### Which tenants

- **Single-tenant authority** (GUID or domain): the token's `tid` must equal it.
  For a domain, the tenant is the one named by the discovery document's issuer.
- **`common`, `organizations`, `consumers`**: the new optional answer
  `oauth_microsoft_allowed_tenants` (comma-separated GUIDs) is required. With it
  empty, no verified identity is minted: the visitor chats anonymously exactly as
  before (a presented token is ignored, not rejected). A malformed list is
  refused at setup and disables minting if it reaches run time raw. With a
  non-empty list, a token from another tenant is a 401.

A presented Microsoft token that fails verification is a 401, never a downgrade
to anonymous.

### Dependency

`rsa 0.9` (default features off, `std` + `sha2`) pulls in `num-bigint-dig`,
`pkcs1`, `rand 0.8` and a second `sha2` major (0.10, RustCrypto's own pairing
for `rsa`). It builds for `wasm32-wasip2`. It is used only to verify. `rsa` 0.9
carries RUSTSEC-2023-0071 (Marvin, a timing side channel in private-key
operations); no private key is ever loaded in the component.

### Known limits

- `email`/`name` are descriptive only; never authorize on them.
- A Direct Line token lives 24 h and refresh re-issues the same claims, so a
  verified identity can outlive the Entra session (id_token ~1 h). The browser
  signs out when its id_token expires, but the server does not re-check.
- The nonce is browser-chosen and bound to the login only as strongly as session
  storage; it is not a server-issued challenge.
- The identity settings are read from the raw setup answers
  (`oauth_enable_microsoft`, `oauth_microsoft_client_id`,
  `oauth_microsoft_authority`, `oauth_microsoft_allowed_tenants`), the shape
  `greentic-setup` writes, as the Greentic SSO fallback does.
- `assets/webchat-gui/oauth.js` is not loaded by any page and was not changed;
  `runtime-bootstrap.js` carries the live flow.

### Cross-repo

- greentic-start: none for `sub`, `iss` and extra claims.
- greentic-runner: only if typed `email`/`name`/`tid` fields on `VerifiedCaller`
  are wanted.
- greentic-designer: the Setup UI renders questions from the emitted answers, so
  the new optional question appears without a designer change.
