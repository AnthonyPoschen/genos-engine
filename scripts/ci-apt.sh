#!/usr/bin/env bash
# Install apt packages, retrying a mirror that is mid-sync.
# security.ubuntu.com can answer `File has unexpected size` for a moment.
# Acquire::Retries covers one fetch. The loop covers the update itself.
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: ci-apt.sh <package>..." >&2
  exit 2
fi

# Lavapipe comes from the kisak Mesa PPA, not the Ubuntu archive. Ubuntu's
# Mesa 26.0.8 segfaulted genos-stress on lavapipe from b151452b on (the
# quit-gap and fill-knockout jobs). Mesa 26.3.1 runs both tests to the end.
# The PPA keeps only its newest Mesa, so CI follows it; the installed version
# is printed below.
wants_mesa=0
for package in "$@"; do
  [[ "$package" == mesa-vulkan-drivers ]] && wants_mesa=1
done
if [[ "$wants_mesa" -eq 1 ]]; then
  for attempt in 1 2 3 4 5; do
    if sudo add-apt-repository -y -n ppa:kisak/kisak-mesa; then
      break
    fi
    if [[ "$attempt" -eq 5 ]]; then
      echo "kisak-mesa PPA failed after 5 attempts" >&2
      exit 1
    fi
    echo "PPA attempt ${attempt} failed" >&2
    sleep $((attempt * 5))
  done
fi

# With the PPA, a source that fails to update fails the attempt. apt-get
# update otherwise treats a Launchpad 503 as a warning and installs Ubuntu's
# Mesa without a word.
update_mode=()
if [[ "$wants_mesa" -eq 1 ]]; then
  update_mode=(-o APT::Update::Error-Mode=any)
fi

for attempt in 1 2 3 4 5; do
  if sudo apt-get update -o Acquire::Retries=5 "${update_mode[@]}" \
    && sudo apt-get install -y -o Acquire::Retries=5 "$@"; then
    if [[ "$wants_mesa" -eq 1 ]]; then
      version=$(dpkg-query -W -f='${Version}' mesa-vulkan-drivers)
      echo "lavapipe: mesa-vulkan-drivers ${version}"
      if [[ "$version" != *kisak* ]]; then
        echo "lavapipe is not the kisak-mesa build" >&2
        exit 1
      fi
    fi
    exit 0
  fi
  echo "apt attempt ${attempt} failed" >&2
  sleep $((attempt * 5))
done

echo "apt failed after 5 attempts" >&2
exit 1
