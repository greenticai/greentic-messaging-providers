# WebChat "Sign in with Microsoft": authority and verified identity

Status: part (a) implemented (`oauth_microsoft_authority`); part (b) is a design
proposal only. Nothing in (b) is implemented, and nothing here was exercised
against a real Entra tenant.

## What exists today

| Piece | Where | Behaviour |
|---|---|---|
| Button config | `gui_core::compose_oauth_providers`, `ops/oauth.rs::handle_auth_config` | Microsoft provider entry with `auth_url`/`token_url` on `login.microsoftonline.com/<authority>/oauth2/v2.0/...`. Was hard-coded to `common`. |
| Browser flow | `assets/webchat-gui/runtime-bootstrap.js`, `oauth.js` | Authorization code + PKCE in the browser; the code is exchanged through `POST /oauth/token-exchange`. |
| Session | `handleTokens` in `runtime-bootstrap.js` | Stores `access_token \|\| id_token` as `token_handle` and decodes `name`/`email`/`picture` from the id_token **without verifying it**, for display only. |
| Direct Line mint | `webchat-directline-core::http::handle_tokens` | Mints a token with `verified: true` only when the request carries `Authorization: Bearer` that verifies; otherwise anonymous. |
| Bearer flag | `runtime-bootstrap.js` (`greentic_bearer`) | Set only on the Greentic SSO fallback path. A Microsoft login never sets it, so `/token` is called with no `Authorization` header. |

Consequence: the Microsoft button is a UI gate. After sign-in the Direct Line
token is anonymous and `extensions.caller` carries `user_verified: false`.

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

## Part (b): carrying the verified identity (proposal, not implemented)

### What greentic-start and the runner already accept

Checked against greentic-start `origin/develop` and greentic-runner
`origin/develop` (read, not run):

- The Direct Line token is HS256 under the provider's `jwt_signing_key`;
  greentic-start re-mints it and carries extra claims through, dropping
  reserved names (`directline_session.rs::RESERVED_CLAIMS`), capped at 4096
  bytes.
- The provider builds the caller block (`caller::caller_block`) from the
  verified token: `user_verified`, `sub`, `team`/`teams`, `iss`, `groups`,
  `role`. greentic-start only strips a client-supplied block on non-provider
  ingress; the provider is the trust boundary.
- The runner's typed readers (`CallerWire`, `ComponentCaller`) decode
  `user_verified`, `sub`, `team`, `groups`, `role` and ignore other keys. Extra
  keys (`email`, `name`, `tid`, `oid`) therefore ride the raw block only; a
  flow reaches them through the raw payload, not through `_caller`. Whether to
  add typed fields upstream is a product choice, not a prerequisite.

So no greentic-start change is required to carry `sub`, `iss` and extra claims.
The work is in this repo.

### Required changes

1. **Browser** (`runtime-bootstrap.js`, `oauth.js`)
   - Generate a `nonce`, send it on the authorize request, keep it in session
     storage.
   - For `provider.id === "microsoft"` keep the **id_token** (audience = our
     client id), not the Graph `access_token`, and send it as the `Bearer` on
     `/token`, as the Greentic path does with `greentic_bearer`. The ID token is
     the only Entra token whose `aud` is our app and whose claims we can verify.
2. **Verifier** (new, in `webchat-directline-core`)
   - Fetch `https://login.microsoftonline.com/<authority>/v2.0/.well-known/openid-configuration`,
     read `jwks_uri`, restrict both to `login.microsoftonline.com` over https,
     cache like the existing JWKS cache with the same refetch cooldown.
   - Verify RS256, pin `alg` (no `none`, no HS*), match `kid`.
   - Check `aud == oauth_microsoft_client_id`, `exp`/`nbf` with a small skew,
     `nonce` against the value the browser sends, and
     `iss == https://login.microsoftonline.com/<tid>/v2.0` using the token's own
     `tid`. With a single-tenant authority `tid` must also equal that tenant.
   - Needs an RSA verification dependency (RustCrypto `rsa` + `sha2`) that builds
     for `wasm32-wasip2`. That is new third-party code inside a signed pack, so
     it needs an explicit decision and a lockfile review.
3. **Mint** (`handle_tokens`): on a verified Entra token, mint with
   `verified: true`, `sub = "ms:<tid>:<oid>"`, and extra claims `idp_iss`,
   `tid`, `oid`, and optionally `email` (`preferred_username`/`email`) and `name`.
   `ms:` keeps it clear of the Greentic `...:users:...` subject shape that
   anonymous callers are already forbidden to claim. `caller_block` then emits
   the extra fields when `verified`.

### Decisions that are not settled by the code

- **Tenant allow-list.** With `common` or `organizations`, any Entra tenant's
  user passes signature verification. `(tid, oid)` is a correct identity, but a
  flow that treats "verified" as "ours" would admit every tenant. Proposal: a
  multi-tenant authority requires an `oauth_microsoft_allowed_tenants` list;
  a single-tenant authority needs none.
- **Subject.** `oid` (stable per tenant) vs `sub` (pairwise per app). Proposal
  `ms:<tid>:<oid>`.
- **Email and name** are not verified identifiers (`email` is optional and
  mutable; never use it for authorization). Carry as descriptive claims only, or
  omit for privacy.
- **Lifetime.** A Direct Line token lives 24h and refresh re-issues the same
  claims, so a verified identity can outlive the Entra session (ID token ~1h).
  The Greentic path has the same property; say so in the operator docs or shorten
  verified-token TTL.
- **Nonce.** A nonce supplied by the browser and checked by the provider binds
  the ID token to the login attempt only as strongly as session storage does; it
  is not a server-issued challenge.
- **Dependency.** Whether to add an RSA crate to the component, or verify at the
  edge (greentic-start) and keep the component dependency-free.

### Cross-repo

- greentic-start: none for `sub`, `iss` and extra claims.
- greentic-runner: only if typed `email`/`name`/`tid` fields on `VerifiedCaller`
  are wanted.
- greentic-designer: the Setup UI renders questions from the emitted answers, so
  the new optional question appears without a designer change.
