#! /usr/bin/sh

set -e  # Exit on any error

BASE_DIR="/opt/sherpa/images/blank_disk"

# create image
qemu-img create -f raw $BASE_DIR/base.img 64M

# FAT16
cp $BASE_DIR/base.img $BASE_DIR/fat16.img
mkfs.fat -F 16 $BASE_DIR/fat16.img

# FAT32
# Cumulus Linux (RIP) needs uses fat32
cp $BASE_DIR/base.img $BASE_DIR/fat32.img
mkfs.fat -F 32 $BASE_DIR/fat32.img

# Junos Disk
# Requires the name to be 'vmm-data'
cp $BASE_DIR/base.img $BASE_DIR/junos.img
mkfs.vfat  -v -n "vmm-data" $BASE_DIR/junos.img

# ISE Disk
# Requires the name to be 'ISE-ZTP'
qemu-img create -f raw $BASE_DIR/ise.img 10M
mkfs.ext4  -L "ISE-ZTP" $BASE_DIR/ise.img

# EXT4 Disks
qemu-img create -f raw $BASE_DIR/ext4-100mb.img 100M
mkfs.ext4 -L "data-disk" $BASE_DIR/ext4-100mb.img

qemu-img create -f raw $BASE_DIR/ext4-200mb.img 200M
mkfs.ext4 -L "data-disk" $BASE_DIR/ext4-200mb.img

qemu-img create -f raw $BASE_DIR/ext4-500mb.img 500M
mkfs.ext4 -L "data-disk" $BASE_DIR/ext4-500mb.img


qemu-img create -f raw $BASE_DIR/ext4-500mb.img 500MB
mkfs.ext4 -L "data-disk" $BASE_DIR/ext4-500mb.img

qemu-img create -f raw $BASE_DIR/ext4-1g.img 1G
mkfs.ext4 -L "data-disk" $BASE_DIR/ext4-1g.img

qemu-img create -f raw $BASE_DIR/ext4-2g.img 2G
mkfs.ext4 -L "data-disk" $BASE_DIR/ext4-2g.img

qemu-img create -f raw $BASE_DIR/ext4-3g.img 3G
mkfs.ext4 -L "data-disk" $BASE_DIR/ext4-3g.img

qemu-img create -f raw $BASE_DIR/ext4-4g.img 4G
mkfs.ext4 -L "data-disk" $BASE_DIR/ext4-4g.img

qemu-img create -f raw $BASE_DIR/ext4-5g.img 5G
mkfs.ext4 -L "data-disk" $BASE_DIR/ext4-5g.img
