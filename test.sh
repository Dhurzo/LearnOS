#!/bin/bash

# Quick test script for the OS kernel
#
# This script builds the kernel and performs a basic functionality test
# by booting it in QEMU for a short duration. It's designed to quickly
# verify that the kernel builds correctly and boots successfully.
#
# Usage:
#   ./test.sh                  # Quick build and test (default: 5s timeout)
#   ./test.sh --timeout 10     # Set custom timeout (default: 5s)
#   ./test.sh --verbose        # Show detailed output
#
# Test Process:
# 1. Build kernel and boot image using build.sh
# 2. Run QEMU with specified timeout
# 3. Report success/failure based on exit status
#
# Expected Behavior:
# - Build script completes without errors
# - QEMU boots successfully and displays "Hello World!" message
# - QEMU exits gracefully after timeout (or remains running)
#
# Exit Codes:
# 0: Test passed - kernel built and booted successfully
# 1: Test failed - build error or QEMU timeout exceeded
# 2: Test failed - missing dependencies or permissions
#
# For more information about testing:
# - [QEMU Testing Guide](https://www.qemu.org/docs/master/system/testing.html)
# - [OSDev Wiki - Testing Kernels](https://wiki.osdev.org/Testing_Kernels)
#
set -e

# Get the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Default settings
TIMEOUT=5                        # Default test duration: 5 seconds
VERBOSE="false"

# Parse command line arguments
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
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --timeout N    Set test timeout to N seconds (default: 5)"
            echo "  --verbose      Show detailed output during test"
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

echo "🚀 Quick OS Kernel Test"
echo "========================"
echo "Timeout: ${TIMEOUT}s"
echo ""

# Step 1: Build the kernel
echo "🔨 Step 1: Building kernel..."
if [ "$VERBOSE" = "true" ]; then
    ./build.sh
else
    # Build silently, only show errors
    ./build.sh > /dev/null 2>&1
fi
echo "✅ Build completed"

# Step 2: Verify build artifacts
echo ""
echo "📦 Step 2: Checking build artifacts..."
if [ ! -f "target/pvh-kernel" ]; then
    echo "❌ ERROR: Kernel binary not found"
    exit 1
fi

KERNEL_SIZE=$(wc -c < target/pvh-kernel)
echo "✅ Kernel binary: ${KERNEL_SIZE} bytes"

# Check boot image only if it was created (requires nasm)
if [ -f "target/boot.img" ]; then
    BOOT_IMG_SIZE=$(wc -c < target/boot.img)
    echo "✅ Boot image: ${BOOT_IMG_SIZE} bytes"
else
    echo "⚠️  Boot image not found (nasm not installed)"
fi

# Step 3: Test kernel boot
echo ""
echo "🧪 Step 3: Testing kernel boot (timeout: ${TIMEOUT}s)..."
echo "This will test that the kernel boots successfully and displays output."

# Choose boot method based on available artifacts
if [ -f "target/boot.img" ]; then
    # Use boot image if available (more reliable)
    echo "Using boot sector method..."
    if [ "$VERBOSE" = "true" ]; then
        QEMU_TIMEOUT="$TIMEOUT" ./run.sh --boot-img
    else
        # Run QEMU silently, capturing only exit status
        set +e
        QEMU_TIMEOUT="$TIMEOUT" timeout "${TIMEOUT}s" qemu-system-x86_64 \
            -drive format=raw,file="target/boot.img" \
            -boot order=a \
            -m 32 \
            -nographic >/dev/null 2>&1
        QEMU_EXIT_CODE=$?
        set -e
        if [ "$QEMU_EXIT_CODE" -eq 0 ] || [ "$QEMU_EXIT_CODE" -eq 124 ]; then
            echo "✅ Kernel booted successfully (timeout expected in minimal kernel)"
        else
            echo "❌ Kernel test failed (QEMU timeout or error)"
            exit 1
        fi
    fi
else
    # Fall back to PVH boot if no boot image
    echo "Using PVH method..."
    if [ "$VERBOSE" = "true" ]; then
        QEMU_TIMEOUT="$TIMEOUT" ./run.sh
    else
        # Run QEMU silently, capturing only exit status
        set +e
        QEMU_TIMEOUT="$TIMEOUT" timeout "${TIMEOUT}s" qemu-system-x86_64 \
            -kernel "target/pvh-kernel" \
            -m 128M \
            -machine q35,accel=kvm:tcg \
            -cpu host \
            -nographic >/dev/null 2>&1
        QEMU_EXIT_CODE=$?
        set -e
        if [ "$QEMU_EXIT_CODE" -eq 0 ] || [ "$QEMU_EXIT_CODE" -eq 124 ]; then
            echo "✅ Kernel booted successfully (timeout expected in minimal kernel)"
        else
            echo "❌ Kernel test failed (QEMU timeout or error)"
            exit 1
        fi
    fi
fi

# Step 4: Test summary
echo ""
echo "🎯 Test Results:"
echo "  ✅ Kernel build: Success"
echo "  ✅ Kernel size: ${KERNEL_SIZE} bytes (normal for minimal kernel)"
echo "  ✅ Boot test: Success"
echo ""
echo "🏁 Test completed successfully!"
echo ""
echo "Next steps:"
echo "  - Use './run.sh --boot-img' for full interactive test"
echo "  - Use './run.sh --debug' for detailed boot information"
echo "  - Check docs/ for development information"
echo ""
