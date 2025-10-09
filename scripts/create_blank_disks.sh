#! /usr/bin/sh

# create image
qemu-img create -f raw base.img 64M

# FAT16
cp base.img fat16.img
mkfs.fat -F 16 fat16.img

# FAT32
cp base.img fat32.img
mkfs.fat -F 32 fat32.img

# Junos
cp base.img junos.img
mkfs.vfat  -v -n "vmm-data"

# EXT4 3GB
qemu-img create -f raw ext4-1g.img 1G
mkfs.ext4 -L "data-disk" ext4-1g.img

qemu-img create -f raw ext4-2g.img 2G
mkfs.ext4 -L "data-disk" ext4-2g.img

qemu-img create -f raw ext4-3g.img 3G
mkfs.ext4 -L "data-disk" ext4-3g.img

qemu-img create -f raw ext4-4g.img 4G
mkfs.ext4 -L "data-disk" ext4-4g.img

qemu-img create -f raw ext4-5g.img 5G
mkfs.ext4 -L "data-disk" ext4-5g.img