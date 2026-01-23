# provision component

Applies provisioning plan actions to the host state store, namespacing config
and secrets entries.

## Operation: apply

Input JSON (string):

```
{
  "plan": {
    "actions": [
      { "type": "config.set", "scope": "tenant", "key": "public_base_url", "value": "https://..." },
      { "type": "secrets.put", "scope": "tenant", "key": "TELEGRAM_BOT_TOKEN", "value": "..." }
    ]
  },
  "dry_run": false
}
```

Output JSON (string):

```
{
  "ok": true,
  "dry_run": false,
  "actions": [
    { "action_type": "config.set", "scope": "tenant", "key": "public_base_url", "status": "ok", "message": null }
  ],
  "summary": {
    "config_keys_written": ["public_base_url"],
    "secret_keys_written": ["TELEGRAM_BOT_TOKEN"]
  }
}
```

Notes:
- `PROVISION_DRY_RUN=1` forces dry-run mode regardless of input.
- Unsupported action types return an error in the action result.
- Writes use the state-store keys `config/<scope>/<key>` and
  `secrets/<scope>/<key>`.
