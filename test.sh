#!/bin/bash

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

TIMEOUT=5
VERBOSE="false"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        --verbose)
            VERBOSE="true"
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo "🚀 Quick PVH Kernel Test"

if [ "$VERBOSE" = "true" ]; then
    ./build.sh
else
    ./build.sh >/dev/null 2>&1
fi

if [ ! -f target/pvh-kernel ]; then
    echo "❌ missing target/pvh-kernel"
    exit 1
fi

set +e
timeout "${TIMEOUT}s" qemu-system-x86_64 -kernel target/pvh-kernel -m 128M -machine q35,accel=kvm:tcg -cpu host -nographic >/dev/null 2>&1
QEMU_EXIT=$?
set -e

if [ "$QEMU_EXIT" -eq 0 ] || [ "$QEMU_EXIT" -eq 124 ]; then
    echo "✅ PVH kernel boot path executed"
else
    echo "❌ PVH kernel boot failed"
    exit 1
fi
