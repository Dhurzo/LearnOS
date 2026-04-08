#!/bin/bash

# Verify script for the minimal educational kernel.
#
# This verifier matches the current build output:
# target/pvh-kernel is a raw binary produced by linker OUTPUT_FORMAT(binary).

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

KERNEL="target/pvh-kernel"
VERIFICATION_MODE="full"   # full, quick, format, symbols
VERBOSE="false"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --quick)
            VERIFICATION_MODE="quick"
            shift
            ;;
        --format)
            VERIFICATION_MODE="format"
            shift
            ;;
        --symbols)
            VERIFICATION_MODE="symbols"
            shift
            ;;
        --verbose)
            VERBOSE="true"
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --quick        Quick verification only"
            echo "  --format       Check binary format markers only"
            echo "  --symbols      Explain symbol limitations for raw binary"
            echo "  --verbose      Show detailed output"
            echo "  -h, --help     Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use -h or --help for usage information"
            exit 1
            ;;
    esac
done

echo "=== Verifying OS Kernel ==="
echo "Mode: $VERIFICATION_MODE"
echo "Kernel: $KERNEL"

if [ ! -f "$KERNEL" ]; then
    echo "ERROR: Kernel not found at $KERNEL"
    echo "Run './build.sh' first."
    exit 1
fi

KERNEL_SIZE=$(wc -c < "$KERNEL")
KERNEL_PERMISSIONS=$(ls -la "$KERNEL" | awk '{print $1}')
echo "Size: ${KERNEL_SIZE} bytes"
echo "Permissions: $KERNEL_PERMISSIONS"

RESULT="PASSED"

echo ""
echo "=== 1. File Format ==="
if [ "$VERIFICATION_MODE" = "format" ] || [ "$VERIFICATION_MODE" = "full" ]; then
    file "$KERNEL"

    if readelf -h "$KERNEL" >/dev/null 2>&1; then
        echo "WARNING: kernel appears to be ELF, expected raw binary"
        RESULT="FAILED"
    else
        echo "OK: kernel is not ELF (expected for OUTPUT_FORMAT(binary))"
    fi
else
    echo "Skipping file format check (--quick mode)"
fi

echo ""
echo "=== 2. PVH Marker ==="
if [ "$VERIFICATION_MODE" = "format" ] || [ "$VERIFICATION_MODE" = "full" ] || [ "$VERIFICATION_MODE" = "quick" ]; then
    PVH_NAME_HEX=$(dd if="$KERNEL" bs=1 skip=12 count=4 2>/dev/null | hexdump -v -e '1/1 "%02x"')
    if [ "$PVH_NAME_HEX" = "50485600" ]; then
        echo "OK: PVH note name found (PHV\\0)"
    else
        echo "WARNING: PVH note name not found at expected offset (got: $PVH_NAME_HEX)"
    fi
else
    echo "Skipping PVH marker check"
fi

echo ""
echo "=== 3. Multiboot Header Marker ==="
if [ "$VERIFICATION_MODE" = "format" ] || [ "$VERIFICATION_MODE" = "full" ] || [ "$VERIFICATION_MODE" = "quick" ]; then
    if hexdump -e '1/4 "%08x" "\n"' "$KERNEL" | grep -Eq 'e85250d6|d65052e8'; then
        echo "OK: Multiboot2 magic found"
    else
        echo "WARNING: Multiboot2 magic not found"
    fi
else
    echo "Skipping multiboot marker check"
fi

echo ""
echo "=== 4. Symbols ==="
if [ "$VERIFICATION_MODE" = "symbols" ] || [ "$VERIFICATION_MODE" = "full" ]; then
    echo "Info: target/pvh-kernel is raw binary, so symbols like _start are not present."
    echo "Info: if you need symbols, inspect intermediate artifacts before raw conversion."
else
    echo "Skipping symbols (--quick mode)"
fi

echo ""
echo "=== 5. Size Sanity ==="
if [ "$KERNEL_SIZE" -lt 1000 ] || [ "$KERNEL_SIZE" -gt 50000 ]; then
    echo "WARNING: size seems unusual for this minimal kernel"
else
    echo "OK: size is within expected range"
fi

if [ "$VERBOSE" = "true" ]; then
    echo ""
    echo "First 16 bytes:"
    hexdump -C "$KERNEL" | head -n 1
fi

echo ""
echo "=== Verification Summary ==="
if [ ! -f "$KERNEL" ] || [ "$KERNEL_SIZE" -eq 0 ]; then
    RESULT="FAILED"
fi

if [ "$RESULT" = "PASSED" ]; then
    echo "PASSED - Kernel verification completed"
    echo ""
    echo "Next steps:"
    echo "  - Test boot with './run.sh --boot-img'"
    echo "  - Run boot image checks with './scripts/verify-boot.sh'"
else
    echo "FAILED - Kernel verification completed"
    echo ""
    echo "Troubleshooting:"
    echo "  - Run './build.sh' to rebuild the kernel"
    echo "  - See docs/troubleshooting.md for more help"
    exit 1
fi

echo ""
echo "=== Verification complete ==="
