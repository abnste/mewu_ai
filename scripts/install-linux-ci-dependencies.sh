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
cat > "$sources" <<'EOF'
deb https://archive.ubuntu.com/ubuntu jammy main restricted universe multiverse
deb https://archive.ubuntu.com/ubuntu jammy-updates main restricted universe multiverse
deb https://archive.ubuntu.com/ubuntu jammy-backports main restricted universe multiverse
deb https://security.ubuntu.com/ubuntu jammy-security main restricted universe multiverse
EOF
options=(-o "Dir::Etc::sourcelist=$sources" -o Dir::Etc::sourceparts=-
  -o Acquire::Retries=2 -o Acquire::https::Timeout=30 -o Acquire::http::Timeout=30
  -o Acquire::Languages=none -o APT::Update::Error-Mode=any)
sudo apt-get "${options[@]}" update
sudo apt-get "${options[@]}" install -y libwebkit2gtk-4.1-dev build-essential libssl-dev librsvg2-dev libayatana-appindicator3-dev patchelf libfuse2 fonts-noto-cjk fonts-dejavu-core xvfb xauth dbus bubblewrap squashfs-tools libtesseract4 tesseract-ocr-eng libgles2 file python3-pyelftools
