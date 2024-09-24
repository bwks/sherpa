# Sherpa

Vagrant re-imagined.

## Why?
I love the workflow of Vagrant. Define an environment and build/destroy it with a few commands. This also made sharing labs with peers much easier. 

Docker came along and it's great when you can get by with only using Containers. I am from a networking background and in that space, VM's are mostly still king.

I am learning Rust and what better way to learn than to build something that you can use. So that is why I am building Sherpa. To help me learn Rust and build a tool that I can use to make my life easier. 

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
  - ovmf
  - virt-manager
```

## Device Support Matrix

- Working - :white_check_mark:
- Planned - :construction:
- Partially Working - :warning:

| Vendor        | Model         | Status             |
| ------------- | ------------- | ------------------ |
| Arista        | vEOS          | :white_check_mark: |
| Cisco         | ASAv          |                    |


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
Juniper allows you to download images, you just need to sign up for an account.no
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

### CentOS
[CentOS Cloud Images](https://cloud.centos.org/)

### RedHat
RedHat requires a subscription. A trial subscription can is available.

[RHEL](https://access.redhat.com/)

[RHEL 9](https://access.redhat.com/downloads/content/479/ver=/rhel---9/9.4/x86_64/product-software)

### SUSE
[OpenSUSE](https://get.opensuse.org/)

[OpenSUSE Leap Micro](https://get.opensuse.org/leapmicro/6.0/)

[SUSE Enterprise](https://www.suse.com/download/sles/)

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
id = "1e18b3bfe149"

devices = [
  { id = 1, name = "dev1", device_model = "linux_fedora" },
  { id = 2, name = "dev2", device_model = "linux_fedora" },
]

connections = [
  { device_a = "dev1", interface_a = 0, device_b = "dev2", interface_b = 0 },
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