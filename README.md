# Sherpa

Vagrant re-imagined.

## Project Goals
- Network based Images are a first class citizen.
- Multi-threaded/Asynchronous.
- Docker image <--> VM network stitching.

## Hypervisor Support
- Initially only KVM/QEMU will be supported


## Goals 
- HTTP/TFTP/PXE boot server options

```
virt-install \
  --connect=qemu:///system \
  --network network=default,model=e1000 \
  --name=iosv \
  --cpu host \
  --arch=x86_64 \
  --vcpus=1 \
  --ram=512 \
  --os-variant=linux2018 \
  --noacpi \
  --virt-type=kvm \
  --watchdog i6300esb,action=reset \
  --disk path=vios-adventerprisek9-m.SPA.159-3.M6/virtioa.qcow2,format=qcow2,device=disk,bus=virtio,cache=writethrough \
  --graphics none \
  --import
```