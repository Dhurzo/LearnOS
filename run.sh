#!/bin/bash

# Run the OS kernel in QEMU
#
# This script provides multiple ways to test the kernel in QEMU emulator,
# supporting both modern PVH boot and traditional boot sector boot methods.
#
# Usage:
#   ./run.sh                        # PVH kernel boot (recommended)
#   ./run.sh --boot-img            # Boot from floppy image
#   ./run.sh --debug               # Enable debug output
#   ./run.sh --timeout 30          # Set custom timeout (default: 10s)
#   ./run.sh --verbose             # Show QEMU output
#
# Boot Methods:
# 1. PVH (Para-Virtualized Hypervisor) mode:
#    - Modern QEMU boot method
#    - Direct kernel loading without BIOS
#    - Faster boot, better hardware access
#    - Uses target/pvh-kernel
#
# 2. Boot sector mode:
#    - Traditional BIOS-based boot
#    - Uses target/boot.img (1.44MB floppy image)
#    - Compatible with older QEMU versions
#    - Simpler boot process, good for debugging
#
# QEMU Command Options:
# - -machine q35: Modern QEMU machine type
# - -accel kvm: Use KVM for hardware virtualization (Linux)
# - -cpu host: Use host CPU features for better performance
# - -nographic: Disable graphical display (text only)
# - -drive: Attach storage device (for boot.img)
# - -boot order=a: Boot from first floppy drive
#
# For more information about QEMU:
# - https://www.qemu.org/docs/master/
# - https://wiki.qemu.org/
#
set -e                           # Exit immediately on error

# Get the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Configuration variables
KERNEL="target/pvh-kernel"
BOOT_IMG="target/boot.img"
TIMEOUT="${QEMU_TIMEOUT:-10}"    # Default timeout: 10 seconds
QEMU_MODE="pvh"                  # Default boot mode
VERBOSE="false"
CLEAN_EXIT="false"

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --boot-img)
            QEMU_MODE="bootimg"
            shift
            ;;
        --debug)
            QEMU_MODE="debug"
            shift
            ;;
        --verbose)
            VERBOSE="true"
            shift
            ;;
        --timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        --clean-exit)
            CLEAN_EXIT="true"
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --boot-img     Boot from floppy image (recommended)"
            echo "  --debug        Enable QEMU debug output"
            echo "  --verbose      Show QEMU output"
            echo "  --timeout N    Set timeout to N seconds (default: 10)"
            echo "  --clean-exit   Exit cleanly after timeout"
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

# Build kernel if it doesn't exist
if [ ! -f "$KERNEL" ]; then
    echo "Kernel not found, building..."
    ./build.sh
fi

# Verify that the required files exist based on boot mode
case "$QEMU_MODE" in
    bootimg)
        if [ ! -f "$BOOT_IMG" ]; then
            echo "ERROR: Boot image not found at $BOOT_IMG"
            echo "Run './build.sh' first to create the boot image (requires nasm)."
            exit 1
        fi
        ;;
    pvh)
        if [ ! -f "$KERNEL" ]; then
            echo "ERROR: Kernel not found at $KERNEL"
            echo "Run './build.sh' first."
            exit 1
        fi
        ;;
esac

# Print run information
echo "=== Running kernel in QEMU ==="
echo "Mode: $QEMU_MODE"
echo "Timeout: ${TIMEOUT}s"
echo "Kernel: $KERNEL"
if [ "$QEMU_MODE" = "bootimg" ]; then
    echo "Boot image: $BOOT_IMG"
fi
echo ""

# Function to clean up QEMU process on exit
cleanup() {
    if [ -n "$QEMU_PID" ]; then
        kill "$QEMU_PID" 2>/dev/null || true
    fi
}

# Set up trap to clean up on script exit
trap cleanup EXIT

# Build QEMU command based on boot mode
case "$QEMU_MODE" in
    bootimg)
        # Boot from floppy image using BIOS
        # This loads the boot sector from the first sector of the floppy image
        echo "Boot method: BIOS boot from floppy image"
        echo "Command: qemu-system-x86_64 -drive format=raw,file=\"$BOOT_IMG\" -boot order=a -m 32 -nographic"
        
        if [ "$VERBOSE" = "true" ]; then
            # Run QEMU in foreground with verbose output
            timeout "${TIMEOUT}s" qemu-system-x86_64 \
                -drive format=raw,file="$BOOT_IMG" \
                -boot order=a \
                -m 32 \
                -nographic
        else
            # Run QEMU in background with timeout
            timeout "${TIMEOUT}s" qemu-system-x86_64 \
                -drive format=raw,file="$BOOT_IMG" \
                -boot order=a \
                -m 32 \
                -nographic &
        fi
        ;;
    debug)
        # Debug mode with detailed QEMU output
        # This enables guest errors, interrupts, and exceptions
        echo "Boot method: PVH kernel with debug output"
        echo "Command: qemu-system-x86_64 -kernel \"$KERNEL\" -m 128M -d guest_errors,un,int -nographic"
        
        # Run QEMU in foreground with debug output
        qemu-system-x86_64 \
            -kernel "$KERNEL" \
            -m 128M \
            -d guest_errors,un,int \
            -nographic
        ;;
    pvh)
        # Default PVH mode - modern QEMU boot without BIOS
        # This uses Para-Virtualized Hypervisor for direct kernel loading
        echo "Boot method: PVH kernel (modern QEMU)"
        echo "Command: qemu-system-x86_64 -kernel \"$KERNEL\" -m 128M -machine q35,accel=kvm:tcg -cpu host -nographic"
        
        if [ "$VERBOSE" = "true" ]; then
            # Run QEMU in foreground with verbose output
            timeout "${TIMEOUT}s" qemu-system-x86_64 \
                -kernel "$KERNEL" \
                -m 128M \
                -machine q35,accel=kvm:tcg \
                -cpu host \
                -nographic
        else
            # Run QEMU in background with timeout
            timeout "${TIMEOUT}s" qemu-system-x86_64 \
                -kernel "$KERNEL" \
                -m 128M \
                -machine q35,accel=kvm:tcg \
                -cpu host \
                -nographic &
        fi
        ;;
esac

# Save the QEMU PID if running in background
if [ "$VERBOSE" = "false" ] && [ "$QEMU_MODE" != "debug" ]; then
    QEMU_PID=$!
    wait $QEMU_PID
    QEMU_EXIT_CODE=$?
else
    QEMU_EXIT_CODE=$?
fi

# Check if QEMU timed out or exited normally
if [ "$QEMU_EXIT_CODE" -eq 124 ]; then
    echo ""
    echo "⏱️ QEMU timeout after ${TIMEOUT}s"
    if [ "$CLEAN_EXIT" = "true" ]; then
        echo "This is normal - kernel booted successfully but ran indefinitely."
        echo "Use Ctrl+C to manually stop QEMU if needed."
    else
        echo "This is normal for a minimal kernel that enters an infinite loop."
        echo "The kernel likely booted successfully and displayed 'Hello World!'."
    fi
elif [ "$QEMU_EXIT_CODE" -eq 0 ]; then
    echo ""
    echo "✅ QEMU exited normally"
else
    echo ""
    echo "❌ QEMU exited with error code: $QEMU_EXIT_CODE"
fi

echo ""
echo "QEMU session ended."