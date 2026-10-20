#!/usr/bin/env bash
# Source this file to load ignored per-machine settings and export the repo root.
_nr_env_script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
_nr_repo_root="$(cd -- "${_nr_env_script_dir}/.." && pwd -P)"
if [[ -f "${_nr_repo_root}/.env.local" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "${_nr_repo_root}/.env.local"
  set +a
fi
export NR_PROJECT_DIR="${NR_PROJECT_DIR:-${_nr_repo_root}}"
unset _nr_env_script_dir _nr_repo_root
