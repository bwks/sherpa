# Building Unikernel Images for Sherpa

This guide covers how to build Unikraft and Nanos unikernel images compatible with
Sherpa. It serves as a reference for manual builds, future Packer automation, and
users building custom images.

## Image Requirements

Sherpa expects unikernel images at a specific path with a specific filename.

| Platform | Boot Mode | Expected Filename | Format |
|----------|-----------|-------------------|--------|
| Unikraft | DirectKernel | `kernel.elf` | ELF binary |
| Nanos | DiskBoot | `disk.qcow2` | QCOW2 disk image |

**Directory structure:**

```
/opt/sherpa/images/
  unikraft_unikernel/
    <version>/
      kernel.elf
  nanos_unikernel/
    <version>/
      disk.qcow2
```

The `<version>` directory name is the version string used to identify the image in
Sherpa (e.g. `1.0.0`, `0.1.5`).

## Default Node Configuration

Both unikernel types share these defaults (configurable per-image in the admin UI):

| Setting | Value |
|---------|-------|
| vCPU | 1 |
| Memory | 512 MiB |
| Network driver | virtio |
| Management interface | eth0 |
| Data interfaces | 0 |
| Architecture | x86_64 |
| Machine type | pc |

## Networking

Unikernels receive their management IP via DHCP. Sherpa configures a static
MAC-to-IP binding on the lab router (dnsmasq), so the unikernel gets a
deterministic address without needing kernel command-line IP injection.

Unikraft pre-built images use DHCP natively. The `kernel_cmdline` field in the
manifest is passed to QEMU's `-append` flag, but for Unikraft the ELF loader
treats the cmdline as an application path (e.g. `/usr/bin/nginx`), not network
parameters.

## Building a Unikraft Image

### Prerequisites

- [kraftkit](https://github.com/unikraft/kraftkit) (`kraft` CLI)
- Docker (used by kraft for builds)

### Build Steps

1. **Initialize a Unikraft application:**

   ```bash
   kraft init -t nginx@1.15 my-nginx
   cd my-nginx
   ```

   Or use an existing application from the [Unikraft catalog](https://github.com/unikraft/catalog).

2. **Build the application:**

   ```bash
   kraft build --plat qemu --arch x86_64
   ```

   This produces a kernel ELF binary. The output path varies by application but
   is typically under `.unikraft/build/`.

3. **Locate the ELF binary:**

   ```bash
   find .unikraft/build -name "*.elf" -o -name "*_qemu-x86_64"
   ```

   The file should be an ELF executable. Verify with:

   ```bash
   file <path-to-elf>
   # Expected: ELF 64-bit LSB executable, x86-64 ...
   ```

4. **Install into Sherpa:**

   ```bash
   sudo mkdir -p /opt/sherpa/images/unikraft_unikernel/1.0.0
   sudo cp <path-to-elf> /opt/sherpa/images/unikraft_unikernel/1.0.0/kernel.elf
   sudo chmod 644 /opt/sherpa/images/unikraft_unikernel/1.0.0/kernel.elf
   ```

5. **Import into Sherpa:**

   Use the Sherpa admin UI to scan for new images, or place the file and
   restart the server to trigger a scan.

### Manifest Example

```toml
name = "unikraft-lab"

[[nodes]]
name = "uk-nginx"
model = "unikraft_unikernel"
kernel_cmdline = "/usr/bin/nginx"
ready_port = 80
```

The `kernel_cmdline` is passed to QEMU as the `-append` argument. For Unikraft
nginx, this tells the ELF loader which application binary to execute.

### What Sherpa Does at Deploy Time

1. Clones `kernel.elf` from the base image directory to the libvirt storage pool
   (`/opt/sherpa/libvirt/images/<node>-<lab_id>.elf`)
2. Generates libvirt domain XML with `<kernel>` pointing to the cloned ELF
3. Assigns a MAC address and creates a DHCP static binding for the management IP
4. Boots the domain via `virsh define` + `virsh start`

## Building a Nanos Image

### Prerequisites

- [ops](https://ops.city) CLI (the Nanos build tool)

### Build Steps

1. **Install ops:**

   ```bash
   curl https://ops.city/get.sh -sSfL | sh
   ```

2. **List available packages:**

   ```bash
   ops pkg list
   ```

3. **Build a package image:**

   ```bash
   ops image create <package> -i <image-name>
   ```

   For example, to build an nginx image:

   ```bash
   ops image create eyberg/nginx -i nanos-nginx
   ```

   This produces a disk image (typically at `~/.ops/images/nanos-nginx.img`).

4. **Convert to QCOW2 (if needed):**

   The `ops` tool may produce a raw `.img` file. Sherpa expects QCOW2 format:

   ```bash
   qemu-img convert -f raw -O qcow2 ~/.ops/images/nanos-nginx.img disk.qcow2
   ```

   If ops already produces a QCOW2 file, skip this step.

5. **Install into Sherpa:**

   ```bash
   sudo mkdir -p /opt/sherpa/images/nanos_unikernel/0.1.5
   sudo cp disk.qcow2 /opt/sherpa/images/nanos_unikernel/0.1.5/disk.qcow2
   sudo chmod 644 /opt/sherpa/images/nanos_unikernel/0.1.5/disk.qcow2
   ```

6. **Import into Sherpa:**

   Use the Sherpa admin UI to scan for new images.

### Manifest Example

```toml
name = "nanos-lab"

[[nodes]]
name = "nanos-nginx"
model = "nanos_unikernel"
ready_port = 8083
```

Nanos images boot from disk (no `kernel_cmdline` needed). The application and its
configuration are baked into the QCOW2 image at build time.

### What Sherpa Does at Deploy Time

1. Clones `disk.qcow2` from the base image directory to the libvirt storage pool
   (`/opt/sherpa/libvirt/images/<node>-<lab_id>.qcow2`)
2. Generates libvirt domain XML with `<boot dev='hd'/>` and a virtio disk device
3. Assigns a MAC address and creates a DHCP static binding for the management IP
4. Boots the domain via `virsh define` + `virsh start`

## Verifying an Image

After placing the image file and importing it, verify:

```bash
# Check the file exists at the expected path
ls -la /opt/sherpa/images/unikraft_unikernel/1.0.0/kernel.elf
ls -la /opt/sherpa/images/nanos_unikernel/0.1.5/disk.qcow2

# For ELF files, confirm format
file /opt/sherpa/images/unikraft_unikernel/1.0.0/kernel.elf

# For QCOW2 files, confirm format
qemu-img info /opt/sherpa/images/nanos_unikernel/0.1.5/disk.qcow2
```

## Packer Integration (Future)

When automating image builds with Packer, the build output should be placed at:

- **Unikraft:** `/opt/sherpa/images/unikraft_unikernel/<version>/kernel.elf`
- **Nanos:** `/opt/sherpa/images/nanos_unikernel/<version>/disk.qcow2`

The Packer post-processor should:

1. Build the unikernel using the platform-specific toolchain
2. Copy/convert the output to the correct filename and format
3. Place it in the versioned directory under `/opt/sherpa/images/`
4. Trigger a Sherpa image scan or call the import API

## Troubleshooting

**Image not detected after placement:**
Run an image scan from the admin UI. Sherpa only discovers images that match the
expected directory structure and filename.

**Unikernel fails to boot:**
- Check `journalctl -u sherpad` for libvirt errors
- Verify the ELF is built for `x86_64` with `file kernel.elf`
- Verify the QCOW2 is valid with `qemu-img check disk.qcow2`
- Ensure the image was built for the QEMU/KVM platform (not Xen, VMware, etc.)

**Network unreachable inside unikernel:**
- Confirm the unikernel supports virtio network devices
- Check that DHCP is working (the sherpa router container must be running)
- For Unikraft, ensure the image was built with network support enabled
