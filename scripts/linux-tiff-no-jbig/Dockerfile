# SPDX-License-Identifier: MPL-2.0
FROM ubuntu@sha256:5ec03bb3441e8b0bf3b4f9cd4629a1ae763010dc3035bb8da3ae6cf026486401 AS builder
ENV DEBIAN_FRONTEND=noninteractive
RUN printf '%s\n' 'deb-src http://archive.ubuntu.com/ubuntu jammy main universe' 'deb-src http://archive.ubuntu.com/ubuntu jammy-updates main universe' 'deb-src http://security.ubuntu.com/ubuntu jammy-security main universe' > /etc/apt/sources.list.d/mewu-source.list
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-debian python3-pyelftools dpkg-dev build-essential cmake ninja-build pkg-config patch libjpeg-dev libz-dev liblzma-dev libzstd-dev libwebp-dev libdeflate-dev && mkdir -p /packet/source /packet/evidence
COPY acquire_source.py build_tiff.py /packet/
ARG TIFF_SOURCE_VERSION=4.3.0-6ubuntu0.13
ENV TIFF_SOURCE_VERSION=${TIFF_SOURCE_VERSION}
RUN python3 /packet/acquire_source.py && python3 /packet/build_tiff.py

FROM builder AS verified
RUN apt-get install -y --no-install-recommends libgdk-pixbuf-2.0-dev libgtk-3-dev xvfb xauth
COPY codec_fixture.c verify_library.py stage_closure.py /packet/
RUN python3 /packet/verify_library.py

FROM scratch AS library-artifact
COPY --from=verified /packet/replacement/lib/libtiff.so.5.7.0 /libtiff.so.5.7.0
COPY --from=verified /packet/source/patched/COPYRIGHT /COPYRIGHT
COPY --from=verified /packet/evidence/ubuntu-source.json /ubuntu-source.json
COPY --from=verified /packet/evidence/upstream-backport.json /upstream-backport.json
COPY --from=verified /packet/evidence/library.json /library.json
COPY --from=verified /packet/evidence/upstream-fax3-issue513-backport.patch /upstream-fax3-issue513-backport.patch
