#! /usr/bin/env python
# Creates a disk image that is compatible with Cisco IOSv and IOSvl2 images.
# The disk image is used to load a config on boot.
# This uses the `mkfs.vfat` command and it is required to be installed

import shutil
import struct
import subprocess
import sys

# Check if mkfs.vfat exists
if not shutil.which("mkfs.vfat"):
    print("Error: mkfs.vfat command not found. Please install dosfstools package.")
    sys.exit(1)

### - Create image - ###
image_size = 131040 * 512  # 131040 sectors * 512 bytes each
image_file_path = "iosv.img"

with open(image_file_path, "wb") as img:
    img.write(b"\x00" * image_size)


boot_sector = bytearray(512)

# OEM ID
boot_sector[3:11] = b"MSDOS5.  "

# Bytes per sector
struct.pack_into("<H", boot_sector, 11, 512)

# Sectors per cluster
boot_sector[13] = 8

# Reserved sectors
struct.pack_into("<H", boot_sector, 14, 8)

# Number of FATs
boot_sector[16] = 2

# Max root directory entries
struct.pack_into("<H", boot_sector, 17, 512)

# Total sectors
struct.pack_into("<I", boot_sector, 19, 131040)

# Media descriptor
boot_sector[21] = 0xF8

# Sectors per FAT
struct.pack_into("<H", boot_sector, 22, 64)

# Sectors per track
struct.pack_into("<H", boot_sector, 24, 63)

# Number of heads
struct.pack_into("<H", boot_sector, 26, 16)

# Hidden sectors
struct.pack_into("<I", boot_sector, 28, 1)

# Volume serial number
struct.pack_into("<I", boot_sector, 39, 0x24E82400)

# Volume label
boot_sector[43:54] = b"\350       "

# FAT signature
boot_sector[510:512] = b"\x55\xaa"

# Write the boot sector to the image
with open(image_file_path, "r+b") as img:
    img.seek(0)
    img.write(boot_sector)

### - Format disk - ###
try:
    subprocess.run(["mkfs.vfat", "-I", image_file_path], check=True)
except subprocess.CalledProcessError as e:
    print(f"Error formatting disk: {e}")
    sys.exit(1)
