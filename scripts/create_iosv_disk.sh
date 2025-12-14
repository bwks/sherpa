#! /usr/bin/bash
# Creates a disk image that is compatible with Cisco IOSv and IOSvl2 images.
# The disk image is used to load a config on boot.
# This uses the `mkfs.vfat` command and it is required to be installed

set -e  # Exit on any error

BASE_DIR="/opt/sherpa/images/blank_disk"

# Check if mkfs.vfat exists
if ! command -v mkfs.vfat &> /dev/null; then
    echo "Error: mkfs.vfat command not found. Please install dosfstools package."
    exit 1
fi

### - Create image - ###
image_size=$((131040 * 512))  # 131040 sectors * 512 bytes each
image_file_path="$BASE_DIR/iosv.img"

# Create empty image file
dd if=/dev/zero of="$image_file_path" bs=512 count=131040 2>/dev/null

### - Create boot sector - ###
# Create a temporary file for the boot sector
boot_sector=$(mktemp)

# Initialize with zeros
dd if=/dev/zero of="$boot_sector" bs=512 count=1 2>/dev/null

# Write boot sector data using printf and dd
{
    # Skip first 3 bytes (jump instruction area)
    printf '\x00\x00\x00'
    # OEM ID (offset 3-10)
    printf 'MSDOS5.0'
    # Bytes per sector (offset 11-12) - 512 in little endian
    printf '\x00\x02'
    # Sectors per cluster (offset 13)
    printf '\x08'
    # Reserved sectors (offset 14-15) - 8 in little endian
    printf '\x08\x00'
    # Number of FATs (offset 16)
    printf '\x02'
    # Max root directory entries (offset 17-18) - 512 in little endian
    printf '\x00\x02'
    # Total sectors (offset 19-22) - 131040 in little endian
    printf '\x00\x00\x02\x00'
    # Media descriptor (offset 21)
    printf '\xf8'
    # Sectors per FAT (offset 22-23) - 64 in little endian
    printf '\x40\x00'
    # Sectors per track (offset 24-25) - 63 in little endian
    printf '\x3f\x00'
    # Number of heads (offset 26-27) - 16 in little endian
    printf '\x10\x00'
    # Hidden sectors (offset 28-31) - 1 in little endian
    printf '\x01\x00\x00\x00'
    # Fill up to offset 39
    printf '\x00\x00\x00\x00\x00\x00\x00\x00'
    # Volume serial number (offset 39-42)
    printf '\x00\x24\xe8\x24'
    # Volume label (offset 43-53)
    printf '\350       '
    # Fill remaining bytes until offset 510
    for i in {54..509}; do
        printf '\x00'
    done
    # FAT signature (offset 510-511)
    printf '\x55\xaa'
} > "$boot_sector"

# Write the boot sector to the image
dd if="$boot_sector" of="$image_file_path" bs=512 count=1 conv=notrunc 2>/dev/null

# Clean up temporary file
rm "$boot_sector"

### - Format disk - ###
if ! mkfs.vfat -I "$image_file_path" &>/dev/null; then
    echo "Error formatting disk with mkfs.vfat"
    exit 1
fi

echo "Successfully created $image_file_path"
