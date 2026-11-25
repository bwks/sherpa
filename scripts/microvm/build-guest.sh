#!/bin/sh
# Build the guest initramfs and install it, with the host kernel, in the microVM folder:
# busybox-static, GNU tar, the Sherpa init and the kernel's network, netfilter and filesystem
# modules with their dependencies. Run as root.
#
# Usage: build-guest.sh [GUEST_DIR [OUT_DIR]]
#   GUEST_DIR  holds init and modules.list (default: this script's folder)
#   OUT_DIR    receives initramfs.img and vmlinuz (default: /opt/sherpa/microvm)
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
GUEST=${1:-$HERE}
OUT=${2:-/opt/sherpa/microvm}
KVER=$(uname -r)
SRC=/lib/modules/$KVER

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
W=$T/root
M=$W/lib/modules/$KVER

mkdir -p "$W/bin" "$W/proc" "$W/sys" "$W/dev" "$W/tmp" "$W/usr/bin" "$M" "$OUT"
cp /bin/busybox "$W/bin/busybox"
cp "$GUEST/init" "$W/init"
cp "$GUEST/modules.list" "$W/modules.list"

# GNU tar keeps file capabilities (security.capability xattrs), which busybox tar drops.
cp /usr/bin/tar "$W/usr/bin/gtar"
for l in $(ldd /usr/bin/tar | grep -o '/[^ ]*'); do
  mkdir -p "$(dirname "$W$l")"
  cp -L "$l" "$W$l"
done

{
  find "$SRC/kernel/net" "$SRC/kernel/fs/overlayfs" "$SRC/kernel/fs/isofs" "$SRC/kernel/fs/fuse" -name '*.ko*'
  find "$SRC/kernel/drivers/net" -maxdepth 1 -name '*.ko*'
  find "$SRC/kernel/drivers/net/bonding" "$SRC/kernel/drivers/net/ipvlan" "$SRC/kernel/drivers/net/vxlan" \
    "$SRC/kernel/drivers/net/team" "$SRC/kernel/drivers/net/wireguard" -name '*.ko*' 2>/dev/null
} | while read -r f; do
  modprobe -S "$KVER" --show-depends "$(basename "$f" | sed 's/\.ko.*//')" 2>/dev/null | awk '$1=="insmod"{print $2}'
done | sort -u | while read -r f; do
  rel=${f#"$SRC"/}
  dst=$M/${rel%.zst}
  mkdir -p "$(dirname "$dst")"
  case "$f" in *.zst) zstd -q -d -c "$f" > "$dst";; *) cp "$f" "$dst";; esac
done

cp "$SRC"/modules.builtin* "$SRC/modules.order" "$M/"
depmod -b "$W" "$KVER"

(cd "$W" && find . | cpio -o -H newc --quiet | gzip -9) > "$T/initramfs.img"
install -m 0644 "$T/initramfs.img" "$OUT/initramfs.img"
install -m 0644 "/boot/vmlinuz-$KVER" "$OUT/vmlinuz"
