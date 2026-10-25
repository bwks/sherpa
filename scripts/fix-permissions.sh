#! /usr/bin/bash

POOL=/opt/sherpa/libvirt/images

# Ownership and modes, leaving storage pool volumes alone: libvirt hands them to the QEMU
# user while a VM runs, and sets their modes when it creates them.
sudo find /opt/sherpa -path "$POOL" -prune -o -exec chown sherpa:sherpa {} \;

# Folders
sudo find /opt/sherpa -path "$POOL" -prune -o -type d -exec chmod 2775 {} \;

# Files
sudo find /opt/sherpa -path "$POOL" -prune -o -type f -exec chmod 664 {} \;

# Lab data: only sherpa, its group and QEMU (kvm)
sudo install -d -o sherpa -g sherpa -m 2750 /opt/sherpa/labs
sudo install -d -o sherpa -g kvm -m 2750 "$POOL"

# Make real (non-symlink) image files read only
sudo find /opt/sherpa/images -type f -exec chmod 444 {} \;

# SSH Keys
sudo chmod 0640 /opt/sherpa/ssh/sherpa_ssh_key
sudo chmod 0644 /opt/sherpa/ssh/sherpa_ssh_key.pub
