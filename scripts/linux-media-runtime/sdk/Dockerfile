# SPDX-License-Identifier: MPL-2.0
FROM ubuntu:22.04@sha256:5ec03bb3441e8b0bf3b4f9cd4629a1ae763010dc3035bb8da3ae6cf026486401
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends build-essential rustc pkg-config python3-pip ninja-build nasm gnupg ca-certificates flex bison libglib2.0-dev libvo-aacenc-dev libmp3lame-dev libpulse-dev libudev-dev libdbus-1-dev libgl-dev libgles-dev libegl-dev libx11-dev libx11-xcb-dev libxext-dev libwayland-dev wayland-protocols libdrm-dev libgbm-dev binutils file patchelf python3-pyelftools && rm -rf /var/lib/apt/lists/*
COPY sdk/requirements.txt /tmp/requirements.txt
RUN pip3 install --no-cache-dir --require-hashes -r /tmp/requirements.txt
COPY sdk/sources.json sdk/fetch.py sdk/build_phases.py sdk/audit.py /opt/mewu-gst-src/
RUN python3 /opt/mewu-gst-src/fetch.py && python3 /opt/mewu-gst-src/build_phases.py verify
ENV PATH=/opt/mewu-gst-next/bin:${PATH} LD_LIBRARY_PATH=/opt/mewu-gst-next/lib PKG_CONFIG_PATH=/opt/mewu-gst-next/lib/pkgconfig GST_PLUGIN_SYSTEM_PATH_1_0=/opt/mewu-gst-next/lib/gstreamer-1.0 GST_PLUGIN_PATH_1_0=/opt/mewu-gst-next/lib/gstreamer-1.0 GST_PLUGIN_SCANNER_1_0=/opt/mewu-gst-next/libexec/gstreamer-1.0/gst-plugin-scanner GST_REGISTRY_1_0=/tmp/mewu-gst-next-registry.bin PIPEWIRE_MODULE_DIR=/opt/mewu-gst-next/lib/pipewire-0.3 SPA_PLUGIN_DIR=/opt/mewu-gst-next/lib/spa-0.2 PIPEWIRE_CONFIG_DIR=/opt/mewu-gst-next/share/pipewire
RUN python3 /opt/mewu-gst-src/build_phases.py ffmpeg
RUN python3 /opt/mewu-gst-src/build_phases.py orc
RUN python3 /opt/mewu-gst-src/build_phases.py openh264
RUN python3 /opt/mewu-gst-src/build_phases.py gstreamer
RUN python3 /opt/mewu-gst-src/build_phases.py gst-plugins-base
RUN python3 /opt/mewu-gst-src/build_phases.py gst-plugins-good
RUN python3 /opt/mewu-gst-src/build_phases.py gst-plugins-bad
RUN python3 /opt/mewu-gst-src/build_phases.py gst-libav
RUN python3 /opt/mewu-gst-src/build_phases.py pipewire
RUN python3 /opt/mewu-gst-src/audit.py
RUN for factory in fdsrc qtdemux queue decodebin videoconvert capsfilter appsink fakesink appsrc audioconvert audioresample openh264enc h264parse mp4mux fdsink avenc_aac aacparse lamemp3enc mpegaudioparse avdec_mp3 avdec_mp3float pipewiresrc pulsesrc; do /opt/mewu-gst-next/bin/gst-inspect-1.0 "$factory" >/dev/null || exit 1; done
COPY export_runtime.py run-with-media /opt/mewu-export/
RUN python3 /opt/mewu-export/export_runtime.py --prefix /opt/mewu-gst-next --destination /opt/mewu-media-runtime --launcher /opt/mewu-export/run-with-media && tar -czf /opt/mewu-media-runtime.tar.gz -C /opt mewu-media-runtime
