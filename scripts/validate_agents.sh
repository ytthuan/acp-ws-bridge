#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AGENTS_MD="${ROOT_DIR}/AGENTS.md"
RUST_AGENT="${ROOT_DIR}/.github/agents/rust-bridge-agent.agent.md"
REVIEW_AGENT="${ROOT_DIR}/.github/agents/code-reviewer.agent.md"

fail() {
  echo "agent validation failed: $*" >&2
  exit 1
}

check_file_exists() {
  local file="$1"
  [[ -f "${file}" ]] || fail "missing required file: ${file#${ROOT_DIR}/}"
}

check_frontmatter_keys() {
  local file="$1"
  local delimiter_count
  delimiter_count="$(grep -c '^---$' "${file}" || true)"
  [[ "${delimiter_count}" -ge 2 ]] || fail "missing frontmatter delimiters in ${file#${ROOT_DIR}/}"

  grep -Eq '^name:\s*\S+' "${file}" || fail "missing 'name' in ${file#${ROOT_DIR}/}"
  grep -Eq '^description:\s*\S+' "${file}" || fail "missing 'description' in ${file#${ROOT_DIR}/}"
  grep -Eq '^tools:\s*\[.+\]' "${file}" || fail "missing 'tools' in ${file#${ROOT_DIR}/}"
  grep -Eq '^model:\s*\S+' "${file}" || fail "missing 'model' in ${file#${ROOT_DIR}/}"
}

check_file_exists "${AGENTS_MD}"
check_file_exists "${RUST_AGENT}"
check_file_exists "${REVIEW_AGENT}"

grep -Fq '.github/agents/rust-bridge-agent.agent.md' "${AGENTS_MD}" \
  || fail "AGENTS.md must reference .github/agents/rust-bridge-agent.agent.md"
grep -Fq '.github/agents/code-reviewer.agent.md' "${AGENTS_MD}" \
  || fail "AGENTS.md must reference .github/agents/code-reviewer.agent.md"

check_frontmatter_keys "${RUST_AGENT}"
check_frontmatter_keys "${REVIEW_AGENT}"

echo "agent validation passed"
