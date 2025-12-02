# Sherpa

Vagrant re-imagined.

## Why?

I love the workflow of Vagrant. Define an environment in a config file and
build/destroy it with a few commands. This also made sharing labs with peers
much easier.

Docker came along and it's great when you can get by with only using Containers.
I am from a networking background and in that space, VM's are mostly still king.

I am learning Rust and what better way to learn than to build something that you
can use. So that is why I am building Sherpa. To help me learn Rust and build and
tool that I can use to make my life easier while im at it.

## Project Goals

- Multi-threaded/Asynchronous.
- VM network <--> Docker image stitching.
- No need for pre-built Vagrant style boxes.
- HTTP/TFTP/PXE/cloud-init boot server options.
- Network based Images are a first class citizen.
- Support for large topology scenarios.

## Hypervisor Support

- Initially only KVM/QEMU will be supported via [libvirt](https://libvirt.org/)
- Potential future [Cloud Hypervisor](https://github.com/cloud-hypervisor/cloud-hypervisor) support.

## Requiremets

Sherpa uses the [Rust Libvirt](https://gitlab.com/libvirt/libvirt-rust) crate which provides bindings to the Libvert C API.

Ensure that you have the required packages installed. There is the list of packages for Ubuntu.

```
  - cpu-checker
  - qemu-kvm
  - libvirt-daemon-system
  - libvirt-clients
  - libvirt-dev
  - bridge-utils
  - virtinst
  - libosinfo-bin # Guest OS KVM info
  - libguestfs-tools # qcow2 disk manipulation
  - ovmf # UEFI Firmware
  - genisoimage # ISO Creation
  - telnet # Connect to VM serial consoles
  - ssh # connect to VM via SSH
  - virt-manager # Optional, gui view of VMs
  - mtools # Copy files to fat formatted disk images
  - e2tools # Copy files to EXT4 formatted disk images
```

```
qemu-img create -f raw usb_config.img 64M
mkfs.vfat usb_config.img
mkfs.fat -F 32 usb_config.img
echo "beuler, beuler" >> config.txt
mcopy -i usb_config.img config.txt ::/
mdir -i usb_config.img ::
7z l usb_config.img
file usb_config.img
```

### SELINUX

```
sudo mkdir /var/lib/libvirt/flatcar-linux/
sudo semanage fcontext -a -t virt_content_t "/var/lib/libvirt/flatcar-linux/flatcar-linux1"
sudo restorecon -R "/var/lib/libvirt/flatcar-linux/flatcar-linux1"
sudo systemctl restart libvirtd.service
```

### AppArmour

```
sudo mkdir /var/lib/libvirt/sherpa/
sudo sh -c 'echo "  # For ignition files" >> /etc/apparmor.d/abstractions/libvirt-qemu'
sudo sh -c 'echo "  /var/lib/libvirt/sherpa/** r," >> /etc/apparmor.d/abstractions/libvirt-qemu'
sudo systemctl restart libvirtd.service
```

### Bridge interface

Create bridge interface to bridge lab-router / lab-vm's to the physical network.

```yaml
network:
    version: 2
    ethernets:
        enp6s0: {}
    bridges:
        br-sherpa0:
            interfaces: [enp6s0]
            parameters:
                stp: false
                forward-delay: 0
```

## Device Support Matrix

- Working - :white_check_mark:
- Planned - :construction:
- Partially Working - :warning:

| Vendor    | Model          | Minimum Tested Version | Status             | ZTP Method       |
| --------- | -------------- | ---------------------- | ------------------ | ----------       |
| Arista    | vEOS           | 4.32.2f                | :white_check_mark: | TFTP             |
| Aruba     | AOS-CX         | 10.07                  | :white_check_mark: | TFTP             |
| Cisco     | ASAv           | 9.20.2                 | :white_check_mark: | CDROM            |
| Cisco     | CSR 1000v      | 17.03.08a              | :white_check_mark: | CDROM            |
| Cisco     | Catalyst 8000v | 17.13.01a              | :white_check_mark: | CDROM            |
| Cisco     | Catalyst 9000v | 17.12.01               | :white_check_mark: | CDROM            |
| Cisco     | XRv 9000       | 7.11.1                 | :white_check_mark: | CDROM            |
| Cisco     | Nexus 9300v    | 10.4.2.f               | :white_check_mark: | CDROM            |
| Cisco     | IOSv           | 159-3.m8               | :white_check_mark: | Disk             |
| Cisco     | IOSv L2        | 20200920               | :white_check_mark: | Disk             |
| Juniper   | vRouter        | 23.4R2-S2.1            | :white_check_mark: | CDROM            |
| Juniper   | vSwitch        | 23.4R2-S2.1            | :white_check_mark: | CDROM            |
| Juniper   | vSRXv3         | 23.2R2.21              | :white_check_mark: | CDROM            |
| Juniper   | vEvolved       | 23.4R2-S2.1            | :white_check_mark: | TFTP             |
| Nvidia    | Cumulus Linux  | 5.9.2                  | :white_check_mark: | USB              |
| Nokia     | SR Linux       | 24.10.1                | :white_check_mark: | TBA              |
| Microsoft | FlatCar Linux  | 3975.2.2               | :white_check_mark: | Ignition         |
| Microsoft | Windows Server | 2024                   | :white_check_mark: | CloudBase-Init   |
| Canonical | Ubuntu Linux   | 24.04                  | :white_check_mark: | Cloud-Init       |
| RedHat    | Fedora Linux   | 40-1.14                | :white_check_mark: | Cloud-Init       |
| SONiC     | Sonic Linux    | 25051122               | :white_check_mark: | TFTP             |

## Obtain Boxes

Boxes are provided by vendors. Links TBU.

### Box location

Base boxes are stored in the `$HOME/.sherpa/boxes` directory. Base boxes are cloned when building an environment.

#### Example Boxes

```
$HOME/.sherpa/boxes
├── arista_veos
│   ├── 4.29.2f
│   │   ├── cdrom.iso
│   │   └── virtioa.qcow2
│   ├── 4.32.2f
│   │   ├── aboot-veos-serial-8.0.2.iso
│   │   └── virtioa.qcow2
│   └── latest
│       ├── aboot.iso
│       ├── virtioa-combined.qcow2
│       └── virtioa.qcow2
├── cisco_cat8000v
│   └── latest
├── cisco_cat9000v
│   ├── 17.12.01
│   │   └── virtioa.qcow2
│   └── latest
│       └── virtioa.qcow2
├── cisco_csr1000v
│   ├── 17.03.08a
│   │   └── virtioa.qcow2
│   └── latest
│       └── virtioa.qcow2
├── cisco_iosv
│   ├── 159-3.m6
│   │   └── virtioa.qcow2
│   ├── 159-3.m8
│   │   └── virtioa.qcow2
│   └── latest
│       └── virtioa.qcow2
├── cisco_iosvl2
│   └── latest
├── cisco_iosxrv9000
│   └── latest
├── cisco_nexus9300v
│   └── latest
├── linux_ubuntu
│   ├── 24.04
│   │   └── virtioa.qcow2
│   └── latest
│       └── virtioa.qcow2
├── nokia_sros
│   └── latest
└── nvidia_cumulus
    ├── 5.4.0
    │   └── virtioa.qcow2
    ├── 5.9.2
    │   └── virtioa.qcow2
    └── latest
        └── virtioa.qcow2
```

### Arista

Arista allows you to download images, you just need to sign up for an account.

#### vEOS

[Arista vEOS](https://www.arista.com/en/support/software-download)

#### ZTP

Notes:

- vEOS requires an IOS boot disk (Aboot).
- vEOS is tested with `Aboot-veos-serial-8.0.2.iso`

#### cEOS

[Arista cEOS](https://www.arista.com/en/support/software-download)

### Cisco

These images are extracted from Cisco Modeling Labs ISO.

- vIOS
- CSR1000v
- ETC... UPDATE

### Juniper

Juniper allows you to download images, you just need to sign up for an account.
[vJunos Router](https://support.juniper.net/support/downloads/?p=vjunos-router)

[vJunos Switch](https://support.juniper.net/support/downloads/?p=vjunos)

### Nokia

### Nvidia

Nvidia allows you to download images, you just need to sign up for an account.

[Cumulus VX](https://www.nvidia.com/en-us/networking/ethernet-switching/cumulus-vx/download/)

### Ubuntu

Ubuntu provides ready built cloud images setup to work with cloud-init.

[Ubuntu Cloud Images](https://cloud-images.ubuntu.com/)

### Fedora

Fedora provides ready built cloud images setup to work with cloud-init.

[Fedora Cloud Images](https://fedoraproject.org/cloud/download)

### Flatcar Linux

[Flatcar Images](https://www.flatcar.org/docs/latest/installing/vms/libvirt/)
wget <https://stable.release.flatcar-linux.net/amd64-usr/current/flatcar_production_qemu_image.img>

### CentOS

Centos provides ready built cloud images setup to work with cloud-init.
[CentOS Cloud Images](https://cloud.centos.org/)

### RedHat

Redhat provides ready built cloud images setup to work with cloud-init.
RedHat also requires a subscription. A trial or developer subscriptions are available.

[RHEL](https://access.redhat.com/)

[RHEL 9](https://access.redhat.com/downloads/content/479/ver=/rhel---9/9.4/x86_64/product-software)

### SUSE

[OpenSUSE](https://get.opensuse.org/)

[OpenSUSE Leap Micro](https://get.opensuse.org/leapmicro/6.0/)

[SUSE Enterprise](https://www.suse.com/download/sles/)

#### Windows

[Windows Server](https://cloudbase.it/windows-cloud-images/)

## Usage

### Initialise Sherpa

Setup Sherpa configurations, boxes directory structure and required Libvirt parameters.

```
Sherpa init
```

###

A manifest defines the device parameters and connection between them.

#### manifest.toml

```toml
devices = [
  { name = "dev01", device_model = "cisco_cat8000v" },
  { name = "dev02", device_model = "arista_veos" },
]

links = [
  { src = "dev01::gig2", dst = "dev02::eth1/1" },
]
```

### Build Environment

Bring up the environment.

```
sherpa up
```

This will bring up the virtual devices and stitch the interfaces together.

### Kill Environment

When done, tear down the environment.

```
sherpa destroy
```

### Import Image

```
sherpa import -s flatcar_production_qemu_image.img -v 4230.2.3 -m flatcar_linux --latest
```

## Troubleshooting

1. qcow2: Image is corrupt; cannot be opened read/write

Run the following command to fix the image

```
qemu-img check -r all  ~/.sherpa/boxes/cisco_cat9000v/latest/virtioa.qcow2
```
