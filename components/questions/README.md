# Questions Component

CLI-first component that emits and validates question specs for provider setup flows.

## Operations

### emit
Input JSON:
```
{"id":"webex-setup","spec_ref":"assets/setup/webex.yaml","context":{"tenant_id":"t1","env":"dev"}}
```

Output JSON string (QuestionSpec):
- id/title
- questions[] with name/title/kind/required/default/help/validate/secret

### validate
Input JSON:
```
{"spec_json":"{...}","answers_json":"{...}"}
```

Output JSON string:
```
{"ok":true,"errors":[]}
```

### example-answers
Input JSON:
```
{"spec_json":"{...}"}
```

Output JSON string:
```
{"webhook_base_url":"","bot_token":""}
```
