use askama::Template;

use crate::model::{
    BiosTypes, ConnectionTypes, CpuArchitecture, Interface, InterfaceTypes, MachineTypes, User,
};

#[derive(Template)]
#[template(
    source = r#"<domain type='kvm' xmlns:qemu='http://libvirt.org/schemas/domain/qemu/1.0'>
  <name>{{ name }}</name>

  <vcpu placement='static'>{{ cpu_count }}</vcpu>

  <memory unit='MiB'>{{ memory }}</memory>

  <features>
    <acpi/>
    <apic/>
    <pae/>
  </features>

  <cpu mode='host-model'>
    <model fallback='allow'/>
    {% if vmx_enabled %}
    <feature name="vmx" policy="require"/>
    {% endif %}
  </cpu>

  <clock offset='utc'>
    <timer name='rtc' tickpolicy='catchup'/>
    <timer name='pit' tickpolicy='delay'/>
    <timer name='hpet' present='no'/>
  </clock>

  <on_poweroff>destroy</on_poweroff>
  <on_reboot>restart</on_reboot>
  <on_crash>destroy</on_crash>

  <os>
    <type arch='{{ cpu_architecture }}' machine='{{ machine_type }}'>hvm</type>
    <osinfo name='generic'/>
    <bootmenu enable='no'/>
    <smbios mode='host'/>
    {% if let Some(cdrom) = cdrom %}
    <boot dev='cdrom'/>
    {% endif %}
    <boot dev='hd'/>
    {% match bios %}
    {%   when BiosTypes::Uefi %}
    <loader readonly='yes' type='pflash'>/usr/share/OVMF/OVMF_CODE.fd</loader>
    <nvram>/var/lib/libvirt/qemu/nvram/{{ name|uppercase }}_VARS.fd</nvram>
    {%   when BiosTypes::SeaBios %}
    <!-- SeaBios - No stanza required -->
    {% endmatch %}
  </os>

  <pm>
    <suspend-to-mem enabled='no'/>
    <suspend-to-disk enabled='no'/>
  </pm>
  
  {% if let Some(ignition_config) = ignition_config %}
  <qemu:commandline>
    <qemu:arg value='-fw_cfg'/>
    <qemu:arg value='name=opt/org.flatcar-linux/config,file={{ crate::core::konst::SHERPA_STORAGE_POOL_PATH }}/{{ name }}.ign'/>
  </qemu:commandline>
  {% endif %}

  <devices>

    <emulator>{{ qemu_bin }}</emulator>

    {% if let Some(cdrom) = cdrom %}
    <disk type='file' device='cdrom'>
      <driver name='qemu' type='raw'/>
      <source file='{{ cdrom }}'/>
      <target dev='sda' bus='sata'/>
      <readonly/>
    </disk>
    {% endif %}

    <disk type='file' device='disk'>
      <driver name='qemu' type='qcow2'/>
      <source file='{{ boot_disk }}'/>
      <target dev='sdb' bus='sata'/>
    </disk>

    {% if let Some(usb_disk) = usb_disk %}
    <disk type='file' device='disk'>
      <driver name='qemu' type='raw'/>
      <source file='{{ usb_disk }}'/>
      <target dev='sdc' bus='usb'/>
    </disk>
    {% endif %}

    <controller type='usb' index='0' model='piix3-uhci'>
      <alias name='usb'/>
    </controller>

    <graphics type='vnc' port='-1' autoport='yes'/>
    <video>
      <model type='cirrus' vram='16384' heads='1'/>
    </video>

    {% for interface in interfaces %}
    {%   match interface.connection_type %}

    {%     when ConnectionTypes::Management %}
    <interface type='network'>
      <alias name='ua-net-{{ name }}-mgmt{{ interface.num }}'/>
      <mtu size='{{ interface.mtu }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source network='{{ crate::core::konst::BOOT_NETWORK_NAME }}'/>
      <model type='{{ interface_type }}'/>
    </interface>

    {%     when ConnectionTypes::Reserved %}
    <interface type='network'>
      <alias name='ua-net-{{ name }}-reserved{{ interface.num }}'/>
      <mtu size='{{ interface.mtu }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source network='{{ crate::core::konst::ISOLATED_NETWORK_NAME }}'/>
      <model type='{{ interface_type }}'/>
      <link state='up'/>
    </interface>

    {%     when ConnectionTypes::Disabled %}
    <interface type='network'>
      <alias name='ua-net-{{ name }}-int{{ interface.num }}'/>
      <mtu size='{{ interface.mtu }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source network='{{ crate::core::konst::ISOLATED_NETWORK_NAME }}'/>
      <model type='{{ interface_type }}'/>
      <link state='down'/>
    </interface>

    {%     when ConnectionTypes::Peer %}
    {%       match interface.connection_map %}
    {%         when Some with (connection_map) %}
    <interface type='udp'>
      <alias name='ua-net-{{ name }}-int{{ interface.num }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source address='{{ connection_map.source_loopback }}' port='{{ connection_map.source_port }}'>
        <local address='{{ connection_map.local_loopback }}' port='{{ connection_map.local_port }}'/>
      </source>
      <model type='{{ interface_type }}'/>
    </interface>
    {%         when None %}
    {%       endmatch %}
    {%   endmatch %}
    {% endfor %}

    <serial type='tcp'>
      <source mode='bind' host='{{ loopback_ipv4 }}' service='{{ telnet_port }}'/>
      <protocol type='telnet'/>
      <target type='isa-serial' port='0'>
        <model name='isa-serial'/>
      </target>
      <alias name='serial0'/>
    </serial>

    <console type='tcp'>
      <source mode='bind' host='{{ loopback_ipv4 }}' service='{{ telnet_port }}'/>
      <protocol type='telnet'/>
      <target type='serial' port='0'/>
      <alias name='serial0'/>
    </console>

    <channel type='unix'>
      <target type='virtio' name='org.qemu.guest_agent.0'/>
    </channel>

    <input type='mouse' bus='ps2'/>
    <input type='keyboard' bus='ps2'/>
  
    <memballoon model='virtio'>
    </memballoon>

    <watchdog model='i6300esb' action='reset'>
      <alias name='watchdog0'/>
    </watchdog>

  </devices>
</domain>"#,
    ext = "xml"
)]
pub struct DomainTemplate {
    pub name: String,
    pub memory: u16,
    pub cpu_architecture: CpuArchitecture,
    pub machine_type: MachineTypes,
    pub cpu_count: u8,
    pub vmx_enabled: bool,
    pub qemu_bin: String,
    pub bios: BiosTypes,
    pub boot_disk: String,
    pub cdrom: Option<String>,
    pub usb_disk: Option<String>,
    pub ignition_config: Option<bool>,
    pub interfaces: Vec<Interface>,
    pub interface_type: InterfaceTypes,
    pub loopback_ipv4: String,
    pub telnet_port: u16,
}

#[derive(Template)]
#[template(
    source = r#"#cloud-config
hostname: {{ hostname }}
fqdn: {{ hostname }}.{{ crate::core::konst::DOMAIN_NAME }}
{%- if password_auth %}
ssh_pwauth: True
{%- endif %}
users:
  {%- for user in users %}
  - name: {{ user.username }}
    {%- if let Some(password) = user.password %}
    plain_text_passwd: {{ password }}
    lock_passwd: false
    {%- endif %}
    ssh_authorized_keys:
      - {{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    {%- if user.sudo %}
    groups: sudo
    {%- endif %}
    shell: /bin/bash
  {%- endfor %}
"#,
    ext = "yml"
)]
pub struct CloudInitTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub password_auth: bool,
}

#[derive(Template)]
#[template(
    source = r#"#!/bin/bash

# CUMULUS-AUTOPROVISIONING

function error() {
  echo -e "\e[0;33mERROR: The ZTP script failed while running the command $BASH_COMMAND at line $BASH_LINENO.\e[0m" >&2
  exit 1
}

# Log all output from this script
exec >> /var/log/autoprovision 2>&1
date "+%FT%T ztp starting script $0"

trap error ERR

#Configs
nv set system hostname {{ hostname }}
nv set service dns default search {{ hostname }}.{{ crate::core::konst::DOMAIN_NAME }}
{%- for user in users %}
nv set system aaa user {{ user.username }}
{%-   if let Some(password) = user.password %}
nv set system aaa user {{ user.username }} password '{{ password }}'
{%- endif %}
nv set system aaa user {{ user.username }} ssh authorized-key {{ user.username }}-ssh-key key {{ user.ssh_public_key.key }}
nv set system aaa user {{ user.username }} ssh authorized-key {{ user.username }}-ssh-key type {{ user.ssh_public_key.algorithm }}
{%-   if user.sudo %}
nv set system aaa user {{ user.username }} role system-admin
{%   endif %}
{%- endfor %}

nv config apply --assume-yes --message "ZTP config"

exit 0
"#,
    ext = "yml"
)]
pub struct CumulusLinuxZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
}

#[derive(Template)]
#[template(
    source = r#"!
hostname {{ hostname }}
dns domain {{ crate::core::konst::DOMAIN_NAME }}
!
no aaa root
!
service routing protocols model multi-agent
!
aaa authorization exec default local
!
{%- for user in users %}
username {{ user.username }} privilege 15{% if let Some(password) = user.password %} secret {{ password }}{% endif %}
username {{ user.username }} ssh-key {{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}
{%- endfor %}
!
interface Management1
   ip address dhcp
!
management api http-commands
   no shutdown
!
end
!
"#,
    ext = "txt"
)]
pub struct AristaVeosZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
}

#[derive(Template)]
#[template(
    source = r#"!
hostname {{ hostname }}
ip domain name {{ crate::core::konst::DOMAIN_NAME }}
no ip domain lookup
crypto key generate rsa modulus 2048
ip ssh version 2
!
aaa new-model
aaa authentication login default local
aaa authorization exec default local
!
{%- for user in users %}
username {{ user.username }} privilege 15{% if let Some(password) = user.password %} secret {{ password }}{% endif %}
{%- endfor %}
!
ip ssh pubkey-chain
{%- for user in users %}
  username {{ user.username }}
   key-hash {{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}
{%- endfor %}
!
!
interface {{ mgmt_interface }}
 ip address dhcp
 negotiation auto
 no shutdown
 exit
!
line con 0
 logging synchronous
 stopbits 1
 exit
!
line vty 0 4
 logging synchronous
 transport input ssh
 exit
!
exit
"#,
    ext = "txt"
)]
pub struct CiscoIosXeZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub mgmt_interface: String,
}

#[derive(Template)]
#[template(
    source = r#"!
hostname {{ hostname }}
ip domain name {{ crate::core::konst::DOMAIN_NAME }}
no ip domain lookup
crypto key generate rsa modulus 2048
ip ssh version 2
!
aaa new-model
aaa authentication login default local
aaa authorization exec default local
!
{%- for user in users %}
username {{ user.username }} privilege 15{% if let Some(password) = user.password %} secret {{ password }}{% endif %}
{%- endfor %}
!
ip ssh pubkey-chain
{%- for user in users %}
  username {{ user.username }}
   key-hash {{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}
{%- endfor %}
!
!
interface {{ mgmt_interface }}
 ip address dhcp
 negotiation auto
 no shutdown
 exit
!
line con 0
 logging synchronous
 stopbits 1
 exit
!
line vty 0 4
 logging synchronous
 transport input ssh
 exit
!
event manager applet ENABLE-MGMT
 event syslog pattern "SYS-5-RESTART"
 action 0 cli command "enable"
 action 1 cli command "conf t"
 action 2 cli command "interface {{ mgmt_interface }}"
 action 3 cli command "no shutdown"
 action 4 cli command "exit"
 action 5 cli command "crypto key generate rsa modulus 2048"
!
exit
"#,
    ext = "txt"
)]
pub struct CiscoIosvZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub mgmt_interface: String,
}

/*

<domain type='kvm'>
  <name>{{ name }}</name>

  <vcpu placement='static'>{{ cpu_count }}</vcpu>

  <memory unit='MiB'>{{ memory }}</memory>

  <os>
    <type arch='{{ cpu_architecture }}' machine='{{ machine_type }}'>hvm</type>
    <boot dev='cdrom'/>
    <boot dev='hd'/>
  </os>

  <features>
    <acpi/>
    <apic/>
    <pae/>
  </features>

  <cpu mode='host-passthrough'>
    <model fallback='allow'/>
  </cpu>

  <clock offset='utc'>
    <timer name='rtc' tickpolicy='catchup'/>
    <timer name='pit' tickpolicy='delay'/>
    <timer name='hpet' present='no'/>
  </clock>

  <on_poweroff>destroy</on_poweroff>
  <on_reboot>restart</on_reboot>
  <on_crash>destroy</on_crash>

  <pm>
    <suspend-to-mem enabled='no'/>
    <suspend-to-disk enabled='no'/>
  </pm>

  <sysinfo type='smbios'>
    <system>
      <entry name='family'>lab</entry>
    </system>
  </sysinfo>

  <devices>

    <emulator>{{ qemu_bin }}</emulator>

    {% if let Some(cdrom) = cdrom %}
    <disk type='file' device='cdrom'>
      <driver name='qemu' type='raw'/>
      <source file='{{ cdrom }}'/>
      <target dev='sda' bus='sata'/>
      <readonly/>
    </disk>
    {% endif %}

    <disk type='file' device='disk'>
      <driver name='qemu' type='qcow2'/>
      <source file='{{ boot_disk }}'/>
      {# <target dev='vda' bus='virtio'/> #}
      <target dev='sdb' bus='sata'/>
    </disk>

    <controller type='pci' index='0' model='pcie-root'>
      <alias name='pcie.0'/>
    </controller>
    <controller type='pci' index='1' model='pcie-root-port'>
      <model name='pcie-root-port'/>
      <target chassis='1' port='0x8'/>
      <alias name='pci.1'/>
      <address type='pci' domain='0x0000' bus='0x00' slot='0x01' function='0x0' multifunction='on'/>
    </controller>
    <controller type='pci' index='2' model='pcie-root-port'>
      <model name='pcie-root-port'/>
      <target chassis='2' port='0x9'/>
      <alias name='pci.2'/>
      <address type='pci' domain='0x0000' bus='0x00' slot='0x01' function='0x1'/>
    </controller>
    <controller type='pci' index='3' model='pcie-root-port'>
      <model name='pcie-root-port'/>
      <target chassis='3' port='0xa'/>
      <alias name='pci.3'/>
      <address type='pci' domain='0x0000' bus='0x00' slot='0x01' function='0x2'/>
    </controller>
    <controller type='pci' index='4' model='pcie-root-port'>
      <model name='pcie-root-port'/>
      <target chassis='4' port='0xb'/>
      <alias name='pci.4'/>
      <address type='pci' domain='0x0000' bus='0x00' slot='0x01' function='0x3'/>
    </controller>
    <controller type='pci' index='5' model='pcie-root-port'>
      <model name='pcie-root-port'/>
      <target chassis='5' port='0xc'/>
      <alias name='pci.5'/>
      <address type='pci' domain='0x0000' bus='0x00' slot='0x01' function='0x4'/>
    </controller>
    <controller type='pci' index='6' model='pcie-root-port'>
      <model name='pcie-root-port'/>
      <target chassis='6' port='0xd'/>
      <alias name='pci.6'/>
      <address type='pci' domain='0x0000' bus='0x00' slot='0x01' function='0x5'/>
    </controller>

    <controller type='usb' index='0'>
      <alias name='usb0'/>
    </controller>

    {% for interface in interfaces %}
    {%   match interface.connection_type %}

    {%     when ConnectionTypes::Management %}
    <interface type='network'>
      <mac address='{{ interface.mac_address }}'/>
      <source network='default'/>
      <model type='{{ interface_type }}'/>
      <link state='down'/>
    </interface>

    {%     when ConnectionTypes::Disabled %}
    <interface type='network'>
      <alias name='ua-net-{{ name }}-{{ interface.num }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source network='default'/>
      <model type='{{ interface_type }}'/>
      <link state='down'/>
    </interface>

    {%     when ConnectionTypes::Peer %}
    {%       match interface.connection_map %}
    {%         when Some with (connection_map) %}
    <interface type='udp'>
      <mac address='{{ interface.mac_address }}'/>
      <source address='{{ connection_map.source_loopback }}' port='{{ connection_map.source_port }}'>
        <local address='{{ connection_map.local_loopback }}' port='{{ connection_map.local_port }}'/>
      </source>
      <model type='{{ interface_type }}'/>
    </interface>
    {%         when None %}
    {%       endmatch %}
    {%   endmatch %}
    {% endfor %}

    <serial type='tcp'>
      <source mode='bind' host='{{ loopback_ipv4 }}' service='{{ telnet_port }}'/>
      <protocol type='telnet'/>
      <target type='isa-serial' port='0'>
        <model name='isa-serial'/>
      </target>
    </serial>

    <console type='tcp'>
      <source mode='bind' host='{{ loopback_ipv4 }}' service='{{ telnet_port }}'/>
      <protocol type='telnet'/>
      <target type='serial' port='0'/>
    </console>

    <channel type='unix'>
      <source mode='bind' path='/var/lib/libvirt/qemu/channel/target/domain-1-iosv/org.qemu.guest_agent.0'/>
      <target type='virtio' name='org.qemu.guest_agent.0' state='disconnected'/>
      <alias name='channel0'/>
      <address type='virtio-serial' controller='0' bus='0' port='1'/>
    </channel>

    <input type='mouse' bus='ps2'>
      <alias name='input0'/>
    </input>

    <input type='keyboard' bus='ps2'>
      <alias name='input1'/>
    </input>
    <audio id='1' type='none'/>

    <watchdog model='i6300esb' action='reset'>
      <alias name='watchdog0'/>
      <address type='pci' domain='0x0000' bus='0x10' slot='0x02' function='0x0'/>
    </watchdog>

    <memballoon model='virtio'>
      <alias name='balloon0'/>
      <address type='pci' domain='0x0000' bus='0x05' slot='0x00' function='0x0'/>
    </memballoon>

    <rng model='virtio'>
      <backend model='random'>/dev/urandom</backend>
      <alias name='rng0'/>
      <address type='pci' domain='0x0000' bus='0x06' slot='0x00' function='0x0'/>
    </rng>

  </devices>
</domain>

*/
/*
    <interface type='bridge'>
      <source bridge='sherpablackhole'/>
      <model type='virtio'/>
    </interface>

    <interface type='network'>
      <alias name='ua-net-{{ name }}-{{ interface.num }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source network='default'/>
      <model type='{{ interface_type }}'/>
      <link state='down'/>
    </interface>

    <serial type='pty'>
      <source path='/dev/pts/4'/>
      <target type='isa-serial' port='0'>
        <model name='isa-serial'/>
      </target>
      <alias name='serial0'/>
    </serial>

    <console type='pty' tty='/dev/pts/4'>
      <source path='/dev/pts/4'/>
      <target type='serial' port='0'/>
      <alias name='serial0'/>
    </console>

    <disk type='file' device='disk'>
      <driver name='qemu' type='qcow2' cache='writethrough'/>
      <source file='{{ boot_disk }}' index='1'/>
      <backingStore/>
      <target dev='vda' bus='virtio'/>
      <alias name='virtio-disk0'/>
      <address type='pci' domain='0x0000' bus='0x04' slot='0x00' function='0x0'/>
    </disk>

    <devices>
      <interface type='network'>
        <source network='default'/>
        <port isolated='yes'/>
      </interface>
    </devices>

    <serial type='tcp'>
      <source mode='bind' host='127.0.0.1' service='64435'/>
      <protocol type='telnet'/>
      <target port='0'/>
      <alias name='serial0'/>
    </serial>

*/
