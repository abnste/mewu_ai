#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Disposable Linux CI only. No product execution or arbitrary SDK prefix.
set -euo pipefail
: "${RUNNER_TEMP:?}" "${GITHUB_ENV:?}"
test ! -e /opt/mewu-gst-next
stage=$(mktemp -d "$RUNNER_TEMP/mewu-linux-media.XXXXXX")
docker build -f scripts/linux-media-runtime/sdk/Dockerfile -t mewu-linux-media-sdk:ci scripts/linux-media-runtime
docker build --target library-artifact -f scripts/linux-tiff-no-jbig/Dockerfile -t mewu-linux-tiff-artifact:ci scripts/linux-tiff-no-jbig
sdk= tiff=
cleanup() { if [ -n "$sdk" ]; then docker rm "$sdk" >/dev/null; fi; if [ -n "$tiff" ]; then docker rm "$tiff" >/dev/null; fi; }
trap cleanup EXIT
sdk=$(docker create mewu-linux-media-sdk:ci)
tiff=$(docker create mewu-linux-tiff-artifact:ci /unused-not-executed)
docker cp "$sdk:/opt/mewu-gst-next" "$stage/mewu-gst-next"
docker cp "$sdk:/opt/mewu-media-runtime" "$stage/runtime"
docker cp "$tiff:/." "$stage/tiff"
sudo cp -a "$stage/mewu-gst-next" /opt/mewu-gst-next
cat >> "$GITHUB_ENV" <<EOF
MEWU_LINUX_MEDIA_RUNTIME=$stage/runtime
MEWU_LINUX_TIFF_ARTIFACT=$stage/tiff
PKG_CONFIG_PATH=/opt/mewu-gst-next/lib/pkgconfig
LD_LIBRARY_PATH=/opt/mewu-gst-next/lib
GST_PLUGIN_PATH_1_0=/opt/mewu-gst-next/lib/gstreamer-1.0
GST_PLUGIN_SYSTEM_PATH_1_0=/opt/mewu-gst-next/lib/gstreamer-1.0
GST_PLUGIN_SCANNER_1_0=/opt/mewu-gst-next/libexec/gstreamer-1.0/gst-plugin-scanner
GST_PTP_HELPER_1_0=/opt/mewu-gst-next/libexec/gstreamer-1.0/gst-ptp-helper
GST_REGISTRY_1_0=$stage/native-gst-registry.bin
PIPEWIRE_MODULE_DIR=/opt/mewu-gst-next/lib/pipewire-0.3
SPA_PLUGIN_DIR=/opt/mewu-gst-next/lib/spa-0.2
PIPEWIRE_CONFIG_DIR=/opt/mewu-gst-next/share/pipewire
PIPEWIRE_CONFIG_NAME=client.conf
EOF