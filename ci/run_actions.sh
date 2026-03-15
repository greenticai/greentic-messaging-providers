#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

ACT_BIN="${ACT_BIN:-act}"
WORKFLOW="${WORKFLOW:-.github/workflows/build-and-publish.yml}"
EVENT="${EVENT:-workflow_dispatch}"
JOB="${JOB:-}"
MATRIX="${MATRIX:-}"
SINGLE="${SINGLE:-}"
QUICK="${QUICK:-0}"
ACT_IMAGE="${ACT_IMAGE:-catthehacker/ubuntu:act-latest}"
PLATFORM="${ACT_PLATFORM:-ubuntu-latest=${ACT_IMAGE}}"
SECRETS_FILE="${ACT_SECRETS_FILE:-.secrets.act}"
ENV_FILE="${ACT_ENV_FILE:-.env.act}"
ACT_VERSION="${ACT_VERSION:-v0.2.82}"
ACT_BIND="${ACT_BIND:-1}"
ACT_TMPDIR="${ACT_TMPDIR:-${ROOT_DIR}/.tmp/act-tmp}"
ACT_ARTIFACT_PATH="${ACT_ARTIFACT_PATH:-${ROOT_DIR}/.tmp/act-artifacts}"
LIST_JOBS="${LIST_JOBS:-0}"
QUICK_TARGET_ROOT="${QUICK_TARGET_ROOT:-${ROOT_DIR}/.tmp/quick-target}"
QUICK_PACKS_DIR="${QUICK_PACKS_DIR:-${ROOT_DIR}/.tmp/quick-packs}"

usage() {
  cat <<'EOF'
Usage:
  ./ci/run_actions.sh [event] [-- ACT_ARGS...]

Defaults:
  event: workflow_dispatch
  workflow: .github/workflows/build-and-publish.yml
  runner image: catthehacker/ubuntu:act-latest

Environment overrides:
  ACT_BIN=act
  WORKFLOW=.github/workflows/build-and-publish.yml
  EVENT=workflow_dispatch
  JOB=validate-pack-inputs
  MATRIX=pack:messaging-dummy
  SINGLE=provision
  QUICK=1
  ACT_IMAGE=catthehacker/ubuntu:act-latest
  ACT_PLATFORM=ubuntu-latest=catthehacker/ubuntu:act-latest
  ACT_SECRETS_FILE=.secrets.act
  ACT_ENV_FILE=.env.act
  ACT_VERSION=v0.2.82
  ACT_BIND=1
  ACT_TMPDIR=.tmp/act-tmp
  ACT_ARTIFACT_PATH=.tmp/act-artifacts
  LIST_JOBS=1
  QUICK_TARGET_ROOT=.tmp/quick-target
  QUICK_PACKS_DIR=.tmp/quick-packs

Examples:
  ./ci/run_actions.sh
  QUICK=1 ./ci/run_actions.sh
  JOB=validate-pack-inputs ./ci/run_actions.sh
  JOB=build-components ./ci/run_actions.sh
  JOB=build-components SINGLE=provision ./ci/run_actions.sh
  JOB=build-components MATRIX=component:provision ./ci/run_actions.sh
  JOB=cargo-test QUICK=1 ./ci/run_actions.sh
  JOB=validate-pack-inputs SINGLE=messaging-dummy ./ci/run_actions.sh
  JOB=validate-pack-inputs MATRIX=pack:messaging-dummy ./ci/run_actions.sh
  LIST_JOBS=1 ./ci/run_actions.sh
EOF
}

prepare_act_runtime() {
  mkdir -p "${ACT_TMPDIR}" "${ACT_ARTIFACT_PATH}"
  find "${ACT_TMPDIR}" -mindepth 1 -maxdepth 1 -exec rm -rf {} + 2>/dev/null || true
  export TMPDIR="${ACT_TMPDIR}"
}

prepare_quick_runtime() {
  mkdir -p \
    "${QUICK_TARGET_ROOT}/components" \
    "${QUICK_TARGET_ROOT}/cargo-component/wasm32-wasip2" \
    "${QUICK_TARGET_ROOT}/cargo-test"
}

prepare_quick_packs_copy() {
  rm -rf "${QUICK_PACKS_DIR}"
  mkdir -p "${QUICK_PACKS_DIR}"
  cp -a "${ROOT_DIR}/packs/." "${QUICK_PACKS_DIR}/"
}

install_act() {
  local gobin

  if ! command -v go >/dev/null 2>&1; then
    echo "Missing 'go'; cannot automatically install act via go install." >&2
    return 1
  fi

  gobin="${GOBIN:-$(go env GOPATH)/bin}"
  echo "Installing act ${ACT_VERSION} via go install"
  GOBIN="${gobin}" go install "github.com/nektos/act@${ACT_VERSION}"
  export PATH="${gobin}:${PATH}"
}

derive_matrix_from_single() {
  if [ -n "${SINGLE}" ] && [ -n "${MATRIX}" ]; then
    echo "Use either SINGLE or MATRIX, not both." >&2
    exit 1
  fi

  if [ -n "${SINGLE}" ]; then
    case "${JOB}" in
      build-components)
        MATRIX="component:${SINGLE}"
        ;;
      validate-pack-inputs|build-packs)
        MATRIX="pack:${SINGLE}"
        ;;
      *)
        echo "SINGLE is only supported for matrix jobs like build-components, validate-pack-inputs, or build-packs." >&2
        exit 1
        ;;
    esac
  fi
}

quick_component() {
  local component="${1:-provision}"
  echo "QUICK mode: building component '${component}' locally"
  TARGET_DIR="${QUICK_TARGET_ROOT}/components" \
    TARGET_COMPONENTS_DIR="${QUICK_TARGET_ROOT}/components" \
    TARGET_DIR_OVERRIDE="${QUICK_TARGET_ROOT}/cargo-component/wasm32-wasip2" \
    bash "./tools/build_components/${component}.sh"
}

quick_cargo_test() {
  echo "QUICK mode: staging local components and running cargo tests locally"
  mkdir -p "${ROOT_DIR}/components/provision" "${ROOT_DIR}/components/questions"
  if [ -f "${QUICK_TARGET_ROOT}/components/provision.wasm" ]; then
    cp -f "${QUICK_TARGET_ROOT}/components/provision.wasm" "${ROOT_DIR}/components/provision/provision.wasm"
  fi
  if [ -f "${QUICK_TARGET_ROOT}/components/questions.wasm" ]; then
    cp -f "${QUICK_TARGET_ROOT}/components/questions.wasm" "${ROOT_DIR}/components/questions/questions.wasm"
  fi
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}" \
    CARGO_TARGET_DIR="${QUICK_TARGET_ROOT}/cargo-test" \
    TARGET_DIR="${QUICK_TARGET_ROOT}/components" \
    TARGET_DIR_OVERRIDE="${QUICK_TARGET_ROOT}/cargo-component/wasm32-wasip2" \
    TARGET_COMPONENTS_DIR="${QUICK_TARGET_ROOT}/components" \
    TARGET_COMPONENTS="${QUICK_TARGET_ROOT}/components" \
    RUSTFLAGS="${RUSTFLAGS:--C debuginfo=0}" \
    RUST_BACKTRACE="${RUST_BACKTRACE:-1}" \
    ./ci/steps/12_cargo_test.sh
}

quick_validate_pack_inputs() {
  local pack="${1:-messaging-dummy}"
  local lock_path="${ROOT_DIR}/packs.lock.json"
  local lock_backup=""
  echo "QUICK mode: preparing artifacts and validating pack '${pack}' locally"
  prepare_quick_packs_copy
  if [ -f "${lock_path}" ]; then
    lock_backup="$(mktemp)"
    cp -f "${lock_path}" "${lock_backup}"
  fi
  restore_quick_lock() {
    if [ -n "${lock_backup}" ] && [ -f "${lock_backup}" ]; then
      cp -f "${lock_backup}" "${lock_path}"
      rm -f "${lock_backup}"
    else
      rm -f "${lock_path}"
    fi
  }
  trap restore_quick_lock RETURN
  TARGET_DIR="${QUICK_TARGET_ROOT}/components" \
    TARGET_COMPONENTS_DIR="${QUICK_TARGET_ROOT}/components" \
    TARGET_COMPONENTS="${QUICK_TARGET_ROOT}/components" \
    TARGET_DIR_OVERRIDE="${QUICK_TARGET_ROOT}/cargo-component/wasm32-wasip2" \
    CARGO_TARGET_DIR="${QUICK_TARGET_ROOT}/cargo-steps" \
    ./ci/steps/03_build_components.sh
  TARGET_COMPONENTS_DIR="${QUICK_TARGET_ROOT}/components" \
    CARGO_TARGET_DIR="${QUICK_TARGET_ROOT}/cargo-steps" \
    ./ci/steps/04_ensure_templates.sh
  TARGET_COMPONENTS_DIR="${QUICK_TARGET_ROOT}/components" \
    GENERATED_PROVIDERS_DIR="${QUICK_TARGET_ROOT}/generated/providers" \
    PACKS_DIR="${QUICK_PACKS_DIR}" \
    CARGO_TARGET_DIR="${QUICK_TARGET_ROOT}/cargo-steps" \
    ./ci/steps/06_gen_flows.sh
  TARGET_COMPONENTS_DIR="${QUICK_TARGET_ROOT}/components" \
    TARGET_COMPONENTS="${QUICK_TARGET_ROOT}/components" \
    PACKS_DIR="${QUICK_PACKS_DIR}" \
    CARGO_TARGET_DIR="${QUICK_TARGET_ROOT}/cargo-steps" \
    PACK_FILTER="${pack}" \
    ./ci/steps/07a_validate_pack_inputs.sh
}

run_quick_mode() {
  derive_matrix_from_single

  case "${JOB:-quick}" in
    quick)
      quick_component "${SINGLE:-provision}"
      quick_cargo_test
      quick_validate_pack_inputs "${QUICK_PACK:-messaging-dummy}"
      ;;
    build-components)
      quick_component "${SINGLE:-${MATRIX#component:}}"
      ;;
    cargo-test)
      quick_cargo_test
      ;;
    validate-pack-inputs)
      quick_validate_pack_inputs "${SINGLE:-${MATRIX#pack:}}"
      ;;
    *)
      echo "QUICK mode is supported for the default quick sequence, build-components, cargo-test, and validate-pack-inputs." >&2
      exit 1
      ;;
  esac
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  usage
  exit 0
fi

extra_args=()
if [ "${1:-}" = "--" ]; then
  shift
elif [ $# -gt 0 ] && [[ "${1}" != -* ]]; then
  EVENT="${1}"
  shift
fi

if [ "${1:-}" = "--" ]; then
  shift
fi

if [ $# -gt 0 ]; then
  extra_args=("$@")
fi

if [ "${QUICK}" = "1" ]; then
  prepare_quick_runtime
  run_quick_mode
  exit 0
fi

if ! command -v "${ACT_BIN}" >/dev/null 2>&1; then
  echo "'${ACT_BIN}' not found; attempting automatic install."
  install_act
fi

if ! command -v "${ACT_BIN}" >/dev/null 2>&1; then
  echo "Failed to install '${ACT_BIN}'. Install nektos/act manually and retry." >&2
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "Missing 'docker'. nektos/act requires a Docker-compatible runtime." >&2
  exit 1
fi

prepare_act_runtime

args=(
  "${EVENT}"
  --workflows "${WORKFLOW}"
  --platform "${PLATFORM}"
  --artifact-server-path "${ACT_ARTIFACT_PATH}"
)

if [ -n "${JOB}" ]; then
  args+=(--job "${JOB}")
fi

derive_matrix_from_single

if [ -n "${MATRIX}" ]; then
  args+=(--matrix "${MATRIX}")
fi

if [ "${ACT_BIND}" = "1" ]; then
  args+=(--bind)
fi

if [ -f "${SECRETS_FILE}" ]; then
  args+=(--secret-file "${SECRETS_FILE}")
fi

if [ -f "${ENV_FILE}" ]; then
  args+=(--env-file "${ENV_FILE}")
fi

args+=("${extra_args[@]}")

if [ "${LIST_JOBS}" = "1" ]; then
  args+=(--list)
fi

echo "Running ${ACT_BIN} ${args[*]}"
echo "Note: act mirrors the GitHub workflow file locally, but it is not a byte-for-byte GitHub runner."
echo "Using TMPDIR=${TMPDIR}"
echo "Using artifact path ${ACT_ARTIFACT_PATH}"

exec "${ACT_BIN}" "${args[@]}"
