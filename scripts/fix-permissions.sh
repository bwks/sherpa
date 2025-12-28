#! /usr/bin/bash

# Folders
sudo find /opt/sherpa -type d -exec chmod 2775 {} \;

# Files
sudo find /opt/sherpa -type f -exec chmod 664 {} \;

# Make real (non-symlink) image files read only
sudo find /opt/sherpa/images -type f -exec chmod 444 {} \;

# SSH Keys
sudo chmod 0640 /opt/sherpa/ssh/sherpa_ssh_key
sudo chmod 0644 /opt/sherpa/ssh/sherpa_ssh_key.pub
