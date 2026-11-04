# Allowed embedder origins (`allowed_embed_origins`)

Optional, non-secret setup answer of `messaging-webchat-gui` that says which
websites may show the hosted chat in a frame (inline `<iframe>` or the chat
bubble, which also renders the chat in an iframe).

| Answer | Meaning |
|---|---|
| unset or blank | Unchanged behaviour: the chat page is served with no framing header. |
| `https://a.example.com, https://b.example.com:8443` | greentic-start serves the chat page with `Content-Security-Policy: frame-ancestors 'self' <origins>` and without `X-Frame-Options`. |

Format: bare origins, separated by commas or whitespace. `https://host[:port]`;
`http://localhost[:port]` is accepted for development. Setup refuses the whole
answer when any entry has a path, query, fragment, credentials, a wildcard, an
IP literal, a non-localhost `http://` scheme or a bad port, and when there are
more than 20 entries. The stored value is lower-cased, de-duplicated and joined
with `,`.

The pack only validates and stores the answer. Serving the header is done by
greentic-start (static route handler); see its `embed_origins` module.

A published pack digest is needed before a deployment can use this: the
designer pins `messaging-webchat-gui` by digest in its providers registry.
