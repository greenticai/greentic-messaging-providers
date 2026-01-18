# Provider Pack Audit (WASM + gtpack + provider extensions)

This audit captures evidence for the packs listed in `docs/audit/packs/matrix.md` and per-pack details in `docs/audit/packs/*.md`.

## Evidence layout
- Extracted gtpack manifests: `docs/audit/packs/_evidence/manifests/` (from `scripts/audit/audit_packs_list.sh`).
- Pack lockfiles: `docs/audit/packs/_evidence/locks/` (from `scripts/audit/audit_packs_list.sh`).
- Extension summaries: `docs/audit/packs/_evidence/extensions/` (from `scripts/audit/audit_packs_extensions.sh`).
- rg outputs: `docs/audit/packs/_evidence/rg/` (from `scripts/audit/audit_packs_rg.sh`).
