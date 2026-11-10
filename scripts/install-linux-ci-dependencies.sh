#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Only the disposable Ubuntu 22.04 release runner; never rewrite host sources.
set -euo pipefail
[[ ${GITHUB_ACTIONS:-} == true && ${RUNNER_OS:-} == Linux ]]
: "${RUNNER_TEMP:?}"
. /etc/os-release
[[ $ID == ubuntu && $VERSION_ID == 22.04 ]]
sources=$(mktemp "$RUNNER_TEMP/mewu-apt.XXXXXX.list")
trap 'rm -- "$sources"' EXIT
chmod 644 "$sources"
options=(-o "Dir::Etc::sourcelist=$sources" -o Dir::Etc::sourceparts=-
  -o Acquire::Retries=2 -o Acquire::https::Timeout=30 -o Acquire::http::Timeout=30
  -o Acquire::ForceIPv4=true -o Acquire::https::Pipeline-Depth=0 -o Acquire::http::Pipeline-Depth=0
  -o Acquire::Languages=none -o APT::Update::Error-Mode=any)
packages=(libwebkit2gtk-4.1-dev build-essential libssl-dev librsvg2-dev libayatana-appindicator3-dev patchelf libfuse2 fonts-noto-cjk fonts-dejavu-core xvfb xauth dbus bubblewrap squashfs-tools libtesseract4 tesseract-ocr-eng libgles2 file python3-pyelftools)
ready=false
# Bound the whole transfer too: a slow stream can evade the inactivity timeout.
# Both endpoints are Ubuntu archives; APT still authenticates every index/package.
for archive in archive.ubuntu.com security.ubuntu.com; do
  printf 'deb https://%s/ubuntu %s main restricted universe multiverse\n' \
    "$archive" jammy "$archive" jammy-updates "$archive" jammy-backports \
    security.ubuntu.com jammy-security > "$sources"
  if sudo timeout --kill-after=10s 90s apt-get "${options[@]}" update && \
     sudo timeout --kill-after=10s 180s apt-get "${options[@]}" install -y --download-only "${packages[@]}"; then
    ready=true
    break
  fi
  echo "Ubuntu package transfer failed or exceeded its time budget: $archive" >&2
done
[[ $ready == true ]]
# Only start dpkg after every archive is available; never time out mid-install.
sudo apt-get "${options[@]}" install -y --no-download "${packages[@]}"
