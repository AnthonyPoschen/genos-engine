#!/usr/bin/env bash
# Install apt packages, retrying a mirror that is mid-sync.
# security.ubuntu.com can answer `File has unexpected size` for a moment.
# Acquire::Retries covers one fetch. The loop covers the update itself.
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: ci-apt.sh <package>..." >&2
  exit 2
fi

for attempt in 1 2 3 4 5; do
  if sudo apt-get update -o Acquire::Retries=5 \
    && sudo apt-get install -y -o Acquire::Retries=5 "$@"; then
    exit 0
  fi
  echo "apt attempt ${attempt} failed" >&2
  sleep $((attempt * 5))
done

echo "apt failed after 5 attempts" >&2
exit 1
