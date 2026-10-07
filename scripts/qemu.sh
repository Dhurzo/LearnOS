#!/bin/bash
# QEMU wrapper: uses the host qemu-system-x86_64 when installed, otherwise an
# Alpine container with qemu pre-installed (learnos-qemu image, built on demand).
# The repo root is bind-mounted at /work, so relative paths (target/...) work.
#
# QEMU_TIMEOUT (seconds) wraps the qemu run with SIGINT so -nographic sessions
# terminate and flush serial output.
set -e

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if command -v qemu-system-x86_64 >/dev/null 2>&1; then
    timeout -s INT "${QEMU_TIMEOUT:-10}" qemu-system-x86_64 "$@"
else
    if ! docker image inspect learnos-qemu >/dev/null 2>&1; then
        printf 'FROM alpine:latest\nRUN apk add --no-cache qemu-system-x86_64 qemu-system-i386\n' \
            | docker build -t learnos-qemu - >/dev/null
    fi
    timeout -s INT "$(( ${QEMU_TIMEOUT:-10} + 5 ))" \
        docker run --rm -v "$REPO:/work" -e QEMU_TIMEOUT learnos-qemu \
        sh -c 'cd /work && timeout -s INT "${QEMU_TIMEOUT:-10}" qemu-system-x86_64 "$@"' _ "$@"
fi
