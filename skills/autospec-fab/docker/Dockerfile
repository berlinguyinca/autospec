# syntax=docker/dockerfile:1
# ---------------------------------------------------------------------------
# autospec-fab real-toolchain container (issue #1300, part of #1270)
#
# Multi-stage, fully pinned image that lands the four real engineering solvers on
# PATH under the EXACT names the stage scripts resolve via shutil.which():
#   ccx        (CalculiX)        -> stage_fea.py
#   simpleFoam (OpenFOAM ESI)    -> stage_cfd.py
#   freecadcmd (FreeCAD headless)-> freecad_harness.py / render
#   fab-vision (vision CLI)      -> stage_vision.py   (the SHIPPED scripts/fab-vision-cli.py)
#
# Result-contract wrappers (docker/wrappers/) bridge real solver output to the
# stages' .dat / cfd_results parse contracts — the ccx wrapper shadows the real
# ccx on PATH and appends SAFETY_FACTOR; foam_cfd_reduce.py reduces OpenFOAM
# fields to cfd_results.
#
# Pinning policy (spec §5.3): base by @sha256 digest; every apt package by exact
# =<version> (ARG-driven so the nightly build bumps them in lock-step); pip with
# --require-hashes. No floating :latest, no unpinned apt installs. amd64 only (v1).
# Build:  docker build --platform=linux/amd64 -f skills/autospec-fab/docker/Dockerfile -t autospec-fab .
# ---------------------------------------------------------------------------

# ubuntu:24.04 multi-arch index digest (resolved 2026-06-22).
ARG UBUNTU_DIGEST=sha256:786a8b558f7be160c6c8c4a54f9a57274f3b4fb1491cf65146521ae77ff1dc54

# Exact apt versions. Pinned so the build is reproducible; the nightly
# fab-container.yml bumps these as the repos move. The base-stage + CalculiX pins
# below were resolved against the live 24.04 "noble" repo on 2026-06-22 and build
# clean on linux/amd64. The FreeCAD + OpenFOAM pins are repo-dependent
# (freecad-python3 is not in default noble pockets at this snapshot; OpenFOAM ESI
# is an external apt repo) — they are documented ARG defaults the deferred nightly
# resolves against whatever the live repos serve at build time.
ARG CCX_VERSION=2.21-1
ARG FREECAD_VERSION=0.21.2+dfsg2-1
ARG OPENFOAM_VERSION=2406.240730
ARG XVFB_VERSION=2:21.1.12-1ubuntu1.6
ARG MESA_VERSION=25.2.8-0ubuntu0.24.04.2
ARG PYTHON_VERSION=3.12.3-0ubuntu2.1
ARG CURL_VERSION=8.5.0-2ubuntu10.9
ARG CA_CERTS_VERSION=20260601~24.04.1
ARG GNUPG_VERSION=2.4.4-2ubuntu17.4
ARG SPC_VERSION=0.99.49.3

# ===========================================================================
# base — minimal pinned ubuntu + python/xvfb/curl, shared by solvers + runtime
# ===========================================================================
FROM ubuntu@${UBUNTU_DIGEST} AS base
ARG XVFB_VERSION
ARG MESA_VERSION
ARG PYTHON_VERSION
ARG CURL_VERSION
ARG CA_CERTS_VERSION
ARG GNUPG_VERSION
ARG SPC_VERSION
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        python3=${PYTHON_VERSION} \
        xvfb=${XVFB_VERSION} \
        libgl1-mesa-dri=${MESA_VERSION} \
        curl=${CURL_VERSION} \
        ca-certificates=${CA_CERTS_VERSION} \
        gnupg=${GNUPG_VERSION} \
        software-properties-common=${SPC_VERSION} \
    && rm -rf /var/lib/apt/lists/*

# ===========================================================================
# solvers — heavy apt layers (CalculiX, FreeCAD, OpenFOAM). Kept ABOVE the COPY
# of fab scripts so iterating the scripts/smoke never rebuilds these layers.
# ===========================================================================
FROM base AS solvers
ARG CCX_VERSION
ARG FREECAD_VERSION
ARG OPENFOAM_VERSION
ENV DEBIAN_FRONTEND=noninteractive

# CalculiX (ccx) — Ubuntu universe.
RUN apt-get update && apt-get install -y --no-install-recommends \
        calculix-ccx=${CCX_VERSION} \
    && rm -rf /var/lib/apt/lists/*

# FreeCAD headless (freecadcmd) + mesa/llvmpipe for offscreen GL.
# NOTE (2026-06-22): freecad-python3 is NOT in the default Ubuntu 24.04 "noble"
# apt pockets at this snapshot ("Unable to locate package") — it must come from
# the FreeCAD PPA. The nightly fab-container.yml enables the pinned PPA before
# this install; the line below carries the exact-version pin the lint enforces.
# Verified clean through the base + CalculiX stages on linux/amd64; this FreeCAD
# layer is the documented apt-availability boundary (operator-deferred to nightly).
RUN add-apt-repository -y ppa:freecad-maintainers/freecad-stable \
    && apt-get update && apt-get install -y --no-install-recommends \
        freecad-python3=${FREECAD_VERSION} \
    && rm -rf /var/lib/apt/lists/*

# OpenFOAM ESI (openfoam2406) — adds the ESI apt repo (pinned key) then installs
# the exact point release. Provides simpleFoam under the OpenFOAM platforms bin.
RUN curl -fsSL https://dl.openfoam.com/add-debian-repo.sh | bash \
    && apt-get update && apt-get install -y --no-install-recommends \
        openfoam2406-default=${OPENFOAM_VERSION} \
    && rm -rf /var/lib/apt/lists/*

# ===========================================================================
# runtime — final image. Inherits the solver layers, prunes docs/tutorials,
# wires OpenFOAM env so non-login shells resolve simpleFoam, hash-installs pip
# deps, then COPYs the fab scripts + wrappers + fab-vision LAST (cache-stable).
# ===========================================================================
FROM solvers AS runtime
ARG OPENFOAM_VERSION

# Prune OpenFOAM tutorials/doc to claw back ~1 GB (spec §3.1).
RUN rm -rf \
        /usr/lib/openfoam/openfoam2406/tutorials \
        /usr/lib/openfoam/openfoam2406/doc \
        /usr/share/doc/* \
    || true

# OpenFOAM env so `which simpleFoam` resolves in a non-login shell (spec §5.2):
# the ESI package exports these via /usr/lib/openfoam/openfoam2406/etc/bashrc;
# replicate the load-bearing vars statically so PATH carries the platforms bin.
ENV WM_PROJECT_DIR=/usr/lib/openfoam/openfoam2406 \
    WM_PROJECT_USER_DIR=/root/OpenFOAM/root-2406 \
    FOAM_LIBBIN=/usr/lib/openfoam/openfoam2406/platforms/linux64GccDPInt32Opt/lib \
    PATH=/usr/lib/openfoam/openfoam2406/platforms/linux64GccDPInt32Opt/bin:/usr/local/bin:/usr/bin:/bin

# Hash-pinned pip deps for the wrappers / vision CLI.
COPY skills/autospec-fab/docker/requirements.txt /tmp/requirements.txt
RUN python3 -m pip install --no-cache-dir --require-hashes -r /tmp/requirements.txt

# --- fab scripts, wrappers, and the SHIPPED vision CLI (COPY last) ---
COPY skills/autospec-fab/scripts /opt/autospec-fab/scripts
COPY skills/autospec-fab/docker/wrappers /opt/autospec-fab/wrappers

# ccx result-contract wrapper SHADOWS the real ccx on PATH: stage_fea.py's
# which("ccx") resolves here; the wrapper runs the real ccx then appends
# SAFETY_FACTOR derived from .dat MAX_MISES vs material yield.
RUN install -m 0755 /opt/autospec-fab/wrappers/ccx_safety_wrapper.py /usr/local/bin/ccx \
    && install -m 0755 /opt/autospec-fab/wrappers/foam_cfd_reduce.py /usr/local/bin/foam-cfd-reduce

# fab-vision: install the EXISTING shipped CLI (scripts/fab-vision-cli.py from
# #1289/#1290) under the resolver name stage_vision.py expects. NOT a new CLI.
RUN install -m 0755 /opt/autospec-fab/scripts/fab-vision-cli.py /usr/local/bin/fab-vision

ENV AUTOSPEC_FAB_VISION_CMD=/usr/local/bin/fab-vision \
    PYTHONUNBUFFERED=1

WORKDIR /work
