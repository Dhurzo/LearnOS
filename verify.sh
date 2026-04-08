#!/bin/bash

set -e

KERNEL="target/pvh-kernel"

if [ ! -f "$KERNEL" ]; then
    echo "ERROR: missing $KERNEL"
    echo "Run ./build.sh first"
    exit 1
fi

echo "=== Verify PVH Kernel ==="
file "$KERNEL"

if ! readelf -h "$KERNEL" >/dev/null 2>&1; then
    echo "❌ kernel is not ELF"
    exit 1
fi

if ! readelf -h "$KERNEL" | grep -q "EXEC"; then
    echo "❌ kernel is not ET_EXEC"
    exit 1
fi

if ! readelf -n "$KERNEL" | grep -q "Xen"; then
    echo "❌ missing Xen PVH note"
    exit 1
fi

echo "✅ ELF header and Xen PVH note present"
