use askama::Template;

use crate::data::{
    BiosTypes, ConnectionTypes, CpuArchitecture, DeviceDisk, DiskBuses, DiskDevices, Interface,
    InterfaceTypes, MachineTypes,
};

#[derive(Debug, Template)]
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
    <boot dev='cdrom'/>
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
    <qemu:arg value='name=opt/org.flatcar-linux/config,file={{ crate::core::konst::SHERPA_STORAGE_POOL_PATH }}/{{ name }}-cfg.ign'/>
  </qemu:commandline>
  {% endif %}

  {# <qemu:commandline>
    <ns0:arg value="-smbios"/>
    <ns0:arg value="type=1,product=VM-VEX"/>
  </qemu:commandline> #}
  
  <devices>

    <emulator>{{ qemu_bin }}</emulator>

    {% for disk in disks %}
    {%   match disk.target_bus %}
    {%     when DiskBuses::Usb %}
    <disk type='file' device='disk'>
      <driver name='{{ disk.driver_name }}' type='{{ disk.driver_format }}'/>
      <source file='{{ disk.src_file }}'/>
      <target dev='{{ disk.target_dev }}' bus='{{ disk.target_bus }}' removable='on'/>
      <address type='usb'/>
    </disk>
    {%     else %}
    {%       match disk.disk_device %}
    {%         when DiskDevices::Cdrom %}

    <disk type='file' device='cdrom'>
      <driver name='{{ disk.driver_name }}' type='{{ disk.driver_format }}'/>
      <source file='{{ disk.src_file }}'/>
      <target dev='{{ disk.target_dev }}' bus='{{ disk.target_bus }}'/>
      <readonly/>
    </disk>
    {%         else %}
    <disk type='file' device='disk'>
      <driver name='{{ disk.driver_name }}' type='{{ disk.driver_format }}'/>
      <source file='{{ disk.src_file }}'/>
      <target dev='{{ disk.target_dev }}' bus='{{ disk.target_bus }}'/>
    </disk>
    {%      endmatch %}
    {%   endmatch %}
    {% endfor %}

    <controller type='usb' index='0' model='qemu-xhci'>
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
      <alias name='ua-net-{{ name }}-mgmt-int{{ interface.num }}'/>
      <mtu size='{{ interface.mtu }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source network='{{ crate::core::konst::SHERPA_MANAGEMENT_NETWORK_NAME }}'/>
      <model type='{{ interface_type }}'/>
    </interface>

    {%     when ConnectionTypes::Reserved %}
    <interface type='network'>
      <alias name='ua-net-{{ name }}-reserved-int{{ interface.num }}'/>
      <mtu size='{{ interface.mtu }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source network='{{ crate::core::konst::SHERPA_ISOLATED_NETWORK_NAME }}'/>
      <model type='{{ interface_type }}'/>
      <link state='up'/>
    </interface>

    {%     when ConnectionTypes::Disabled %}
    <interface type='network'>
      <alias name='ua-net-{{ name }}-disabled-int{{ interface.num }}'/>
      <mtu size='{{ interface.mtu }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source network='{{ crate::core::konst::SHERPA_ISOLATED_NETWORK_NAME }}'/>
      <model type='{{ interface_type }}'/>
      <link state='down'/>
    </interface>

    {%     when ConnectionTypes::Peer %}
    {%       match interface.interface_connection %}
    {%         when Some with (interface_connection) %}
    <interface type='udp'>
      <alias name='ua-net-{{ name }}-p2p-int{{ interface.num }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source address='{{ interface_connection.source_loopback }}' port='{{ interface_connection.source_port }}'>
        <local address='{{ interface_connection.local_loopback }}' port='{{ interface_connection.local_port }}'/>
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
    // pub boot_disk: String,
    // pub disk2: Option<String>,
    // pub cdrom: Option<String>,
    // pub usb_disk: Option<String>,
    pub disks: Vec<DeviceDisk>,
    pub ignition_config: Option<bool>,
    pub interfaces: Vec<Interface>,
    pub interface_type: InterfaceTypes,
    pub loopback_ipv4: String,
    pub telnet_port: u16,
}

/*
    {#
    {% if let Some(cdrom) = cdrom %}
    <disk type='file' device='cdrom'>
      <driver name='qemu' type='raw'/>
      <source file='{{ cdrom }}'/>
      {% match machine_type %}
      {%   when MachineTypes::Pc %}
      <target dev="hda" bus="ide"/>
      {%   else %}
      <target dev="sda" bus="sata"/>
      {% endmatch %}
      <readonly/>
    </disk>
    {% endif %}

    <disk type='file' device='disk'>
      <driver name='qemu' type='qcow2'/>
      <source file='{{ boot_disk }}'/>
      <target dev='vda' bus='virtio'/>
    </disk>

    <disk type='file' device='disk'>
      <driver name='qemu' type='raw'/>
      <source file='/tmp/disk2.img'/>
      <target dev='vdb' bus='virtio'/>
    </disk>
    <disk type='file' device='disk'>
      <driver name='qemu' type='raw'/>
      <source file='/tmp/disk3.img'/>
      <target dev='vdc' bus='virtio'/>
    </disk>

    {% if let Some(disk2) = disk2 %}
    <disk type='file' device='disk'>
      <driver name='qemu' type='raw'/>
      <source file='{{ disk2 }}'/>
      <target dev='vdd' bus='virtio'/>
    </disk>
    {% endif %}

    {% if let Some(usb_disk) = usb_disk %}
    <disk type='file' device='disk'>
      <driver name='qemu' type='raw'/>
      <source file='{{ usb_disk }}'/>
      <target dev='sdd' bus='usb' removable='on'/>
      <address type='usb' bus='0' port='1'/>
    </disk>
    {% endif %}
    #}
*/
