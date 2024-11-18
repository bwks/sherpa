use askama::Template;

use crate::data::{
    BiosTypes, ConnectionTypes, CpuArchitecture, Interface, InterfaceTypes, MachineTypes,
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

  {# <qemu:commandline>
    <ns0:arg value="-smbios"/>
    <ns0:arg value="type=1,product=VM-VEX"/>
  </qemu:commandline> #}
  
  <devices>

    <emulator>{{ qemu_bin }}</emulator>

    {% if let Some(cdrom) = cdrom %}
    <disk type='file' device='cdrom'>
      <driver name='qemu' type='raw'/>
      <source file='{{ cdrom }}'/>
      {% match machine_type %}
      {%   when MachineTypes::PcI440Fx_4_2 %}
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
      <target dev='sdb' bus='sata'/>
    </disk>

    {% if let Some(usb_disk) = usb_disk %}
    <disk type='file' device='disk'>
      <driver name='qemu' type='raw'/>
      <source file='{{ usb_disk }}'/>
      <target dev='sdc' bus='usb' removable='on'/>
      <address type='usb' bus='0' port='1'/>
    </disk>
    {% endif %}

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
      <alias name='ua-net-{{ name }}-mgmt{{ interface.num }}'/>
      <mtu size='{{ interface.mtu }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source network='{{ crate::core::konst::SHERPA_MANAGEMENT_NETWORK_NAME }}'/>
      <model type='{{ interface_type }}'/>
    </interface>

    {%     when ConnectionTypes::Reserved %}
    <interface type='network'>
      <alias name='ua-net-{{ name }}-reserved{{ interface.num }}'/>
      <mtu size='{{ interface.mtu }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source network='{{ crate::core::konst::SHERPA_ISOLATED_NETWORK_NAME }}'/>
      <model type='{{ interface_type }}'/>
      <link state='up'/>
    </interface>

    {%     when ConnectionTypes::Disabled %}
    <interface type='network'>
      <alias name='ua-net-{{ name }}-int{{ interface.num }}'/>
      <mtu size='{{ interface.mtu }}'/>
      <mac address='{{ interface.mac_address }}'/>
      <source network='{{ crate::core::konst::SHERPA_ISOLATED_NETWORK_NAME }}'/>
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
