.section .multiboot_header
# Multiboot2 Header
#
# This section contains a Multiboot2 header that allows the kernel to be
# loaded by multiboot-compatible bootloaders like GRUB. The Multiboot2
# specification provides a standardized interface between bootloaders and
# operating system kernels.
#
# Format Specification (Multiboot2, see Section 3.1 of the spec):
# - The header must be aligned to 8 bytes
# - It must be embedded in the OS image (typically in the first 32768 bytes)
# - Required fields: magic, architecture, header_length, checksum
# - Followed by optional tag entries (none in this minimal kernel)
#
# For more information:
# - https://www.gnu.org/software/grub/manual/multiboot2/multiboot.html
#
# Header Fields (each .long is a 32-bit / 4-byte field):
#
#   magic          = 0xe85250d6  (Multiboot2 magic number)
#   architecture   = 0x00000000  (0 = i386 / x86 protected mode)
#   header_length  = 24          (total size of this header in bytes:
#                                  4 fields * 4 bytes = 16 bytes for the
#                                  fixed part, plus 8 bytes for the
#                                  end tag = 24 bytes)
#   checksum       = -(magic + architecture + header_length)
#                  = -(0xe85250d6 + 0x00000000 + 0x00000018)
#                  = 0x17daf2b2  (must be 0 when summed with the other 3 fields)
#
# After the fixed fields, a terminating end tag (type=0, size=8) signals
# that no additional optional tags follow. This is required by Multiboot2
# even when no optional tags are present.
#
# End tag:
#   type = 0x00000000  (end tag marker)
#   size = 0x00000008  (size of this tag = 8 bytes)

.long 0xe85250d6      # magic        - Multiboot2 signature
.long 0x00000000      # architecture - 0 = i386 protected mode
.long 0x00000018      # header_length - 24 bytes (16 fixed + 8 end tag)
.long 0x17daf2b2      # checksum - -(0xe85250d6 + 0x0 + 0x18) = 0x17daf2b2
# End tag (required by Multiboot2 even with no optional tags)
.long 0x00000000      # type = 0 (end tag)
.long 0x00000008      # size = 8 bytes
