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
  - virt-manager
```

## Usage
### Initialise Sherpa
Setup Sherpa configurations and required Libvirt parameters
```
Sherpa init
```

###  
A manifest defines the device parameters and connection between them.

#### manifest.toml
```toml
id = "1e18b3bfe149"

devices = [
  { id = 1, name = "dev1", device_model = "nvidia_cumulus" },
  { id = 2, name = "dev2", device_model = "nvidia_cumulus" },
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