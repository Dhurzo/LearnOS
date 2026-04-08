#!/bin/bash

# Run the kernel in QEMU using the real Rust PVH path only.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

KERNEL="target/pvh-kernel"
TIMEOUT="${QEMU_TIMEOUT:-10}"
TIMEOUT_SET="false"
VERBOSE="false"
GUI_MODE="true"
INTERACTIVE_MODE="false"
DEBUG_MODE="false"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug)
            DEBUG_MODE="true"
            shift
            ;;
        --verbose)
            VERBOSE="true"
            shift
            ;;
        --timeout)
            TIMEOUT="$2"
            TIMEOUT_SET="true"
            shift 2
            ;;
        --gui)
            GUI_MODE="true"
            shift
            ;;
        --interactive)
            INTERACTIVE_MODE="true"
            VERBOSE="true"
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --debug        Enable QEMU debug output"
            echo "  --verbose      Show QEMU output"
            echo "  --gui          Use QEMU VGA window (disable -nographic)"
            echo "  --interactive  Run in foreground without timeout"
            echo "  --timeout N    Set timeout to N seconds (default: 10)"
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

if [ "$TIMEOUT_SET" = "false" ] && [ "$INTERACTIVE_MODE" = "false" ]; then
    INTERACTIVE_MODE="true"
    VERBOSE="true"
    GUI_MODE="true"
fi

if [ "$INTERACTIVE_MODE" = "true" ] && [ "$GUI_MODE" != "true" ]; then
    GUI_MODE="true"
fi

if [ ! -f "$KERNEL" ]; then
    echo "Kernel not found, building..."
    ./build.sh
fi

if [ ! -f "$KERNEL" ]; then
    echo "ERROR: Kernel not found at $KERNEL"
    echo "Run './build.sh' first."
    exit 1
fi

echo "=== Running kernel in QEMU ==="
echo "Mode: pvh"
if [ "$INTERACTIVE_MODE" = "true" ]; then
    echo "Timeout: disabled (interactive mode)"
else
    echo "Timeout: ${TIMEOUT}s"
fi
echo "Kernel: $KERNEL"
if [ "$GUI_MODE" = "true" ]; then
    echo "Display: GUI VGA window"
else
    echo "Display: -nographic"
fi

echo ""

if [ "$DEBUG_MODE" = "true" ]; then
    echo "Boot method: PVH kernel with debug output"
    qemu-system-x86_64 -kernel "$KERNEL" -m 128M -machine q35,accel=kvm:tcg -cpu host -d guest_errors,un,int -nographic
    exit 0
fi

QEMU_CMD=(qemu-system-x86_64 -kernel "$KERNEL" -m 128M -machine q35,accel=kvm:tcg -cpu host -vga std)
if [ "$GUI_MODE" != "true" ]; then
    QEMU_CMD+=( -nographic )
fi

if [ "$VERBOSE" = "true" ]; then
    if [ "$INTERACTIVE_MODE" = "true" ]; then
        "${QEMU_CMD[@]}"
    else
        timeout "${TIMEOUT}s" "${QEMU_CMD[@]}"
    fi
else
    timeout "${TIMEOUT}s" "${QEMU_CMD[@]}" &
    QEMU_PID=$!
    wait $QEMU_PID
fi
