#!/bin/bash

# Verify boot sector for the OS kernel project
#
# This script specifically checks the boot sector format and compatibility
# with BIOS boot methods. It examines the boot image to ensure it follows
# the proper boot sector format for 512-byte sectors.
#
# Usage:
#   ./verify-boot.sh              # Full verification
#   --quick                       # Quick check only
#   --verbose                    # Show detailed output
#
# Boot Sector Format:
# - Total size: 512 bytes (1 sector)
# - Load address: 0x7C00 (31KB)
# - Signature: 0xAA55 at offset 510-511
# - Code: Assembly code for video setup and message display
#
# Verification Process:
# 1. Check boot image exists
# 2. Verify file size (should be 512 bytes)
# 3. Check boot sector signature
# 4. Examine boot sector content
# 5. Validate assembly code structure
#
# Boot Sector Requirements:
# - Proper BIOS interrupt usage
# - Correct video mode initialization
# - Valid character encoding
# - Infinite loop to prevent reboot
#
# For more information about boot sectors:
# - [OSDev Wiki - Boot Sector](https://wiki.osdev.org/Boot_Sector)
# - [BIOS Interrupt Reference](https://www.osdever.net/bb/viewtopic.php?f=1&t=2173)
# - [PC Boot Specification](http://www.phoenix.com/nrf/nrf0112.pdf)
#
set -e

# Get the parent directory (this script is in scripts/)
SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$SCRIPT_DIR"

# Configuration
KERNEL="target/pvh-kernel"
BOOT_IMG="target/boot.img"
VERIFICATION_MODE="full"
VERBOSE="false"

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --quick)
            VERIFICATION_MODE="quick"
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

echo "=== Verifying kernel boot format ==="
echo "Mode: $VERIFICATION_MODE"
echo "Boot image: $BOOT_IMG"

# Check if boot image exists
if [ ! -f "$BOOT_IMG" ]; then
    echo "❌ ERROR: Boot image not found at $BOOT_IMG"
    echo "Run './build.sh' first to create the boot image (requires nasm)."
    exit 1
fi

# Get file size and permissions
BOOT_IMG_SIZE=$(wc -c < "$BOOT_IMG")
BOOT_IMG_PERMISSIONS=$(ls -la "$BOOT_IMG" | awk '{print $1}')
echo "Size: ${BOOT_IMG_SIZE} bytes"
echo "Permissions: $BOOT_IMG_PERMISSIONS"

echo ""
echo "=== 1. Basic Format Check ==="
if [ "$VERIFICATION_MODE" = "full" ]; then
    # Check file type
    echo "File type:"
    file "$BOOT_IMG"
    
    # Verify size (should be multiple of 512 bytes)
    if [ "$((BOOT_IMG_SIZE % 512))" -eq 0 ]; then
        echo "✅ File size is multiple of 512 bytes"
    else
        echo "❌ File size is not multiple of 512 bytes"
    fi
    
    # Check if it's a valid disk image
    if fdisk -l "$BOOT_IMG" >/dev/null 2>&1; then
        echo "✅ Valid disk image recognized by fdisk"
    else
        echo "⚠️  Not recognized as standard disk image (may be normal for boot-only image)"
    fi
else
    echo "Skipping basic format check (--quick mode)"
fi

echo ""
echo "=== 2. Boot Sector Signature ==="
if [ "$VERIFICATION_MODE" = "full" ]; then
    # Check for boot sector signature (0xAA55 at offset 510)
    echo "Checking boot sector signature..."
    SIGNATURE=$(dd if="$BOOT_IMG" bs=1 count=2 skip=510 2>/dev/null | hexdump -e '1/1 "%02x"')
    
    if [ "$SIGNATURE" = "aa55" ] || [ "$SIGNATURE" = "55aa" ]; then
        echo "✅ Boot sector signature found: 0xAA55"
    else
        echo "❌ Boot sector signature not found: got 0x$SIGNATURE"
    fi
    
    # Check for boot sector code
    echo ""
    echo "First 64 bytes of boot sector (hex):"
    dd if="$BOOT_IMG" bs=1 count=64 2>/dev/null | hexdump -C | head -n 2
    
    # Check for known patterns in boot code
    echo ""
    echo "Checking for known boot patterns..."
    if dd if="$BOOT_IMG" bs=1 count=512 2>/dev/null | strings | grep -i "Hello World"; then
        echo "✅ 'Hello World!' message found in boot sector"
    else
        echo "⚠️  'Hello World!' message not found (may be normal)"
    fi
else
    echo "Checking boot sector signature (quick)..."
    SIGNATURE=$(dd if="$BOOT_IMG" bs=1 count=2 skip=510 2>/dev/null | hexdump -e '1/1 "%02x"')
    if [ "$SIGNATURE" = "aa55" ] || [ "$SIGNATURE" = "55aa" ]; then
        echo "✅ Boot sector signature: 0xAA55"
    else
        echo "❌ Boot sector signature: 0x$SIGNATURE"
    fi
fi

echo ""
echo "=== 3. Boot Sector Content ==="
if [ "$VERIFICATION_MODE" = "full" ]; then
    # Examine boot sector assembly code
    echo "Disassembling boot sector (first 64 bytes)..."
    objdump -D -b binary -m i386 "$BOOT_IMG" -s --start-address=0 --stop-address=64 2>/dev/null || \
    echo "  (objdump failed - trying hexdump)"
    
    # Check for BIOS interrupt calls
    echo ""
    echo "Checking for BIOS interrupt patterns..."
    if dd if="$BOOT_IMG" bs=1 count=512 2>/dev/null | hexdump -C | grep -q "cd 10"; then
        echo "✅ BIOS interrupt 0x10 found"
    else
        echo "⚠️  BIOS interrupt 0x10 not found"
    fi
    
    # Check for video mode setup
    echo ""
    echo "Checking for video mode setup..."
    if dd if="$BOOT_IMG" bs=1 count=512 2>/dev/null | hexdump -C | grep -E "b[0-9].*03"; then
        echo "✅ Video mode setup (0x03) found"
    else
        echo "⚠️  Video mode setup not found"
    fi
    
    # Check for infinite loop
    echo ""
    echo "Checking for infinite loop pattern..."
    if dd if="$BOOT_IMG" bs=1 count=512 2>/dev/null | hexdump -C | grep -q "eb fe"; then
        echo "✅ Infinite loop pattern (JMP $) found"
    else
        echo "⚠️  Infinite loop pattern not found"
    fi
else
    echo "Skipping detailed content analysis (--quick mode)"
fi

echo ""
echo "=== 4. Message Content ==="
if [ "$VERIFICATION_MODE" = "full" ]; then
    # Extract and display the message
    echo "Extracting boot sector message..."
    MESSAGE=$(dd if="$BOOT_IMG" bs=1 count=512 2>/dev/null | strings | grep -v "^\s*$" | head -n 5)
    if [ -n "$MESSAGE" ]; then
        echo "Message content:"
        echo "  $MESSAGE"
    else
        echo "No readable message found in boot sector"
    fi
    
    # Check for ASCII characters
    echo ""
    echo "Checking ASCII character encoding..."
    ASCII_COUNT=$(dd if="$BOOT_IMG" bs=1 count=512 2>/dev/null | tr -cd '[[:print:]]' | wc -c)
    if [ "$ASCII_COUNT" -gt 0 ]; then
        echo "✅ Found $ASCII_COUNT printable ASCII characters"
    else
        echo "⚠️  No printable ASCII characters found"
    fi
else
    echo "Extracting message (quick)..."
    MESSAGE=$(dd if="$BOOT_IMG" bs=1 count=512 2>/dev/null | strings | head -n 1)
    if [ -n "$MESSAGE" ]; then
        echo "Message: $MESSAGE"
    else
        echo "No message found"
    fi
fi

echo ""
echo "=== 5. Sector Structure ==="
if [ "$VERIFICATION_MODE" = "full" ]; then
    # Check sector boundaries
    echo "Sector structure analysis:"
    SECTOR_COUNT=$((BOOT_IMG_SIZE / 512))
    echo "  Total sectors: $SECTOR_COUNT"
    
    # Check if first sector contains boot code
    if [ "$SECTOR_COUNT" -gt 0 ]; then
        echo "  First sector (512 bytes): Contains boot code and signature"
    fi
    
    # Check remaining sectors (should be zeros for minimal boot image)
    if [ "$SECTOR_COUNT" -gt 1 ]; then
        SECOND_SECTOR_CONTENT=$(dd if="$BOOT_IMG" bs=512 count=1 skip=1 2>/dev/null | hexdump -e '1/1 "%02x"')
        ZERO_COUNT=$(echo "$SECOND_SECTOR_CONTENT" | tr -d '0' | wc -c)
        TOTAL_CHARS=$((SECTOR_COUNT - 1) * 512 * 2)  # 2 hex chars per byte
        
        if [ "$ZERO_COUNT" -eq "$TOTAL_CHARS" ]; then
            echo "  Other sectors: All zeros (normal for boot-only image)"
        else
            echo "  Other sectors: Contains non-zero data"
        fi
    fi
else
    echo "Sector structure (quick):"
    SECTOR_COUNT=$((BOOT_IMG_SIZE / 512))
    echo "  Total sectors: $SECTOR_COUNT"
fi

echo ""
echo "=== Boot Sector Verification Summary ==="
# Determine overall verification result
RESULT="✅ PASSED"

# Check for critical issues
if [ ! -f "$BOOT_IMG" ]; then
    RESULT="❌ FAILED"
elif [ "$BOOT_IMG_SIZE" -eq 0 ]; then
    RESULT="❌ FAILED - Empty file"
elif [ "$((BOOT_IMG_SIZE % 512))" -ne 0 ]; then
    RESULT="❌ FAILED - Invalid sector size"
else
    # Check boot signature
    SIGNATURE=$(dd if="$BOOT_IMG" bs=1 count=2 skip=510 2>/dev/null | hexdump -e '1/1 "%02x"')
    if [ "$SIGNATURE" != "aa55" ] && [ "$SIGNATURE" != "55aa" ]; then
        RESULT="❌ FAILED - Invalid boot signature"
    fi
fi

echo "$RESULT - Boot sector verification completed"

if [ "$RESULT" = "✅ PASSED" ]; then
    echo ""
    echo "Next steps:"
    echo "  - Test boot with './run.sh --boot-img'"
    echo "  - Run full verification with './verify.sh'"
    echo "  - See docs/ for boot sector information"
else
    echo ""
    echo "Troubleshooting:"
    echo "  - Run './build.sh' to rebuild the boot image"
    echo "  - Check if nasm is installed: which nasm"
    echo "  - See docs/troubleshooting.md for more help"
fi

echo ""
echo "=== Boot verification complete ==="
