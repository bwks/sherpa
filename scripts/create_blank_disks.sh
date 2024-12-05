#! /usr/bin/sh

# create image
qemu-img create -f raw base.img 64M

# FAT16
cp base.img fat16.img
mkfs.fat -F 16 fat16.img

# FAT32
cp base.img fat32.img
mkfs.fat -F 32 fat16.img