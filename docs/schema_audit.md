# Component Operation Schema Audit

This audit checks every `components/**/component.manifest.json` for operation input/output schemas and flags those that are empty or unconstrained.

Definition of meaningful schema (for this audit):
- not `{}` or an empty object
- not `{"type":"object"}` with no properties/required and `additionalProperties` omitted/true
- referenced `$ref` schemas must also be meaningful

## Results

| component_id | operation | input_schema_ok | output_schema_ok | notes/fixes |
| --- | --- | --- | --- | --- |
| ai.greentic.component-templates | text | yes | yes | schemas under `components/templates/schemas/io/` and `components/ai.greentic.component-templates/schemas/io/` |
| messaging-ingress-slack | (none) | n/a | n/a | no operations declared |
| messaging-ingress-teams | (none) | n/a | n/a | no operations declared |
| messaging-ingress-telegram | (none) | n/a | n/a | no operations declared |
| messaging-ingress-whatsapp | (none) | n/a | n/a | no operations declared |
| messaging-provider-dummy | (none) | n/a | n/a | no operations declared |
| messaging-provider-email | (none) | n/a | n/a | no operations declared |
| messaging-provider-slack | (none) | n/a | n/a | no operations declared |
| messaging-provider-teams | (none) | n/a | n/a | no operations declared |
| messaging-provider-telegram | (none) | n/a | n/a | no operations declared |
| messaging-provider-webchat | (none) | n/a | n/a | no operations declared |
| messaging-provider-webex | (none) | n/a | n/a | no operations declared |
| messaging-provider-whatsapp | (none) | n/a | n/a | no operations declared |
| ai.greentic.component-provision | apply | yes | yes | schemas under `components/provision/schemas/` |
| ai.greentic.component-questions | emit | yes | yes | schemas under `components/questions/schemas/` |
| ai.greentic.component-questions | validate | yes | yes | schemas under `components/questions/schemas/` |
| ai.greentic.component-questions | example-answers | yes | yes | schemas under `components/questions/schemas/` |
| secrets-probe | (none) | n/a | n/a | no operations declared |
| slack | (none) | n/a | n/a | no operations declared |
| teams | (none) | n/a | n/a | no operations declared |
| telegram | (none) | n/a | n/a | no operations declared |
| ai.greentic.component-templates | text | yes | yes | duplicate manifest id (components/templates and components/ai.greentic.component-templates) |
| webchat | (none) | n/a | n/a | no operations declared |
| webex | (none) | n/a | n/a | no operations declared |
| whatsapp | (none) | n/a | n/a | no operations declared |

## Notes

- The questions/provision/components templates schemas were updated to be minimal-but-structured and referenced from their manifests.
- Components without `operations` are listed as `n/a` (no operation schemas to validate).
