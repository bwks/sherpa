use std::net::Ipv4Addr;

use askama::Template;

use template::{PaloAltoPanosBootstrapTemplate, PaloAltoPanosZtpTemplate};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_INIT_IPV4: &str = "\
type=static
ip-address=172.20.0.10
default-gateway=172.20.0.1
netmask=255.255.255.0
hostname=panos01
dns-primary=172.20.0.1
dns-secondary=8.8.8.8";

const EXPECTED_BOOTSTRAP_IPV4: &str = r#"<?xml version="1.0"?>
<config version="11.0.0" urldb="paloaltonetworks">
  <mgt-config>
    <users>
      <entry name="admin">
        <phash>HASH</phash>
        <permissions>
          <role-based>
            <superuser>yes</superuser>
          </role-based>
        </permissions>
      </entry>
      <entry name="alice">
        <phash>HASH</phash>
        <permissions>
          <role-based>
            <superuser>yes</superuser>
          </role-based>
        </permissions>
        <public-key>OWNERKEY1</public-key>
        <public-key>OWNERKEY2</public-key>
      </entry>
    </users>
    <password-complexity>
      <enabled>yes</enabled>
      <minimum-length>8</minimum-length>
    </password-complexity>
  </mgt-config>
  <shared>
    <application/>
    <application-group/>
    <service/>
    <service-group/>
    <botnet>
      <configuration>
        <http>
          <dynamic-dns>
            <enabled>yes</enabled>
            <threshold>5</threshold>
          </dynamic-dns>
          <malware-sites>
            <enabled>yes</enabled>
            <threshold>5</threshold>
          </malware-sites>
          <recent-domains>
            <enabled>yes</enabled>
            <threshold>5</threshold>
          </recent-domains>
          <ip-domains>
            <enabled>yes</enabled>
            <threshold>10</threshold>
          </ip-domains>
          <executables-from-unknown-sites>
            <enabled>yes</enabled>
            <threshold>5</threshold>
          </executables-from-unknown-sites>
        </http>
        <other-applications>
          <irc>yes</irc>
        </other-applications>
        <unknown-applications>
          <unknown-tcp>
            <destinations-per-hour>10</destinations-per-hour>
            <sessions-per-hour>10</sessions-per-hour>
            <session-length>
              <maximum-bytes>100</maximum-bytes>
              <minimum-bytes>50</minimum-bytes>
            </session-length>
          </unknown-tcp>
          <unknown-udp>
            <destinations-per-hour>10</destinations-per-hour>
            <sessions-per-hour>10</sessions-per-hour>
            <session-length>
              <maximum-bytes>100</maximum-bytes>
              <minimum-bytes>50</minimum-bytes>
            </session-length>
          </unknown-udp>
        </unknown-applications>
      </configuration>
      <report>
        <topn>100</topn>
        <scheduled>yes</scheduled>
      </report>
    </botnet>
  </shared>
  <devices>
    <entry name="localhost.localdomain">
      <network>
        <interface>
          <ethernet/>
        </interface>
        <profiles>
          <monitor-profile>
            <entry name="default">
              <interval>3</interval>
              <threshold>5</threshold>
              <action>wait-recover</action>
            </entry>
          </monitor-profile>
        </profiles>
        <ike>
          <crypto-profiles>
            <ike-crypto-profiles>
              <entry name="default">
                <encryption>
                  <member>aes-128-cbc</member>
                  <member>3des</member>
                </encryption>
                <hash>
                  <member>sha1</member>
                </hash>
                <dh-group>
                  <member>group2</member>
                </dh-group>
                <lifetime>
                  <hours>8</hours>
                </lifetime>
              </entry>
              <entry name="Suite-B-GCM-128">
                <encryption>
                  <member>aes-128-cbc</member>
                </encryption>
                <hash>
                  <member>sha256</member>
                </hash>
                <dh-group>
                  <member>group19</member>
                </dh-group>
                <lifetime>
                  <hours>8</hours>
                </lifetime>
              </entry>
              <entry name="Suite-B-GCM-256">
                <encryption>
                  <member>aes-256-cbc</member>
                </encryption>
                <hash>
                  <member>sha384</member>
                </hash>
                <dh-group>
                  <member>group20</member>
                </dh-group>
                <lifetime>
                  <hours>8</hours>
                </lifetime>
              </entry>
            </ike-crypto-profiles>
            <ipsec-crypto-profiles>
              <entry name="default">
                <esp>
                  <encryption>
                    <member>aes-128-cbc</member>
                    <member>3des</member>
                  </encryption>
                  <authentication>
                    <member>sha1</member>
                  </authentication>
                </esp>
                <dh-group>group2</dh-group>
                <lifetime>
                  <hours>1</hours>
                </lifetime>
              </entry>
              <entry name="Suite-B-GCM-128">
                <esp>
                  <encryption>
                    <member>aes-128-gcm</member>
                  </encryption>
                  <authentication>
                    <member>none</member>
                  </authentication>
                </esp>
                <dh-group>group19</dh-group>
                <lifetime>
                  <hours>1</hours>
                </lifetime>
              </entry>
              <entry name="Suite-B-GCM-256">
                <esp>
                  <encryption>
                    <member>aes-256-gcm</member>
                  </encryption>
                  <authentication>
                    <member>none</member>
                  </authentication>
                </esp>
                <dh-group>group20</dh-group>
                <lifetime>
                  <hours>1</hours>
                </lifetime>
              </entry>
            </ipsec-crypto-profiles>
            <global-protect-app-crypto-profiles>
              <entry name="default">
                <encryption>
                  <member>aes-128-cbc</member>
                </encryption>
                <authentication>
                  <member>sha1</member>
                </authentication>
              </entry>
            </global-protect-app-crypto-profiles>
          </crypto-profiles>
        </ike>
        <qos>
          <profile>
            <entry name="default">
              <class-bandwidth-type>
                <mbps>
                  <class>
                    <entry name="class1">
                      <priority>real-time</priority>
                    </entry>
                    <entry name="class2">
                      <priority>high</priority>
                    </entry>
                    <entry name="class3">
                      <priority>high</priority>
                    </entry>
                    <entry name="class4">
                      <priority>medium</priority>
                    </entry>
                    <entry name="class5">
                      <priority>medium</priority>
                    </entry>
                    <entry name="class6">
                      <priority>low</priority>
                    </entry>
                    <entry name="class7">
                      <priority>low</priority>
                    </entry>
                    <entry name="class8">
                      <priority>low</priority>
                    </entry>
                  </class>
                </mbps>
              </class-bandwidth-type>
            </entry>
          </profile>
        </qos>
        <virtual-router>
          <entry name="default">
            <protocol>
              <bgp>
                <enable>no</enable>
                <dampening-profile>
                  <entry name="default">
                    <cutoff>1.25</cutoff>
                    <reuse>0.5</reuse>
                    <max-hold-time>900</max-hold-time>
                    <decay-half-life-reachable>300</decay-half-life-reachable>
                    <decay-half-life-unreachable>900</decay-half-life-unreachable>
                    <enable>yes</enable>
                  </entry>
                </dampening-profile>
              </bgp>
            </protocol>
          </entry>
        </virtual-router>
      </network>
      <deviceconfig>
        <system>
          <type>
            <static/>
          </type>
          <update-server>updates.paloaltonetworks.com</update-server>
          <update-schedule>
            <threats>
              <recurring>
                <weekly>
                  <day-of-week>wednesday</day-of-week>
                  <at>01:02</at>
                  <action>download-only</action>
                </weekly>
              </recurring>
            </threats>
          </update-schedule>
          <timezone>US/Pacific</timezone>
          <service>
            <disable-telnet>yes</disable-telnet>
            <disable-http>no</disable-http>
          </service>
          <ip-address>172.20.0.10</ip-address>
          <hostname>panos01</hostname>
          <netmask>255.255.255.0</netmask>
          <default-gateway>172.20.0.1</default-gateway>
          <dns-setting>
            <servers>
              <primary>172.20.0.1</primary>
            </servers>
          </dns-setting>
        </system>
        <setting>
          <config>
            <rematch>yes</rematch>
          </config>
          <management>
            <hostname-type-in-syslog>FQDN</hostname-type-in-syslog>
            <initcfg>
              <type>
                <static/>
              </type>
              <ip-address>172.20.0.10</ip-address>
              <default-gateway>172.20.0.1</default-gateway>
              <netmask>255.255.255.0</netmask>
              <hostname>panos01</hostname>
              <dns-primary>172.20.0.1</dns-primary>
            </initcfg>
          </management>
          <auto-mac-detect>yes</auto-mac-detect>
        </setting>
      </deviceconfig>
      <vsys>
        <entry name="vsys1">
          <application/>
          <application-group/>
          <zone/>
          <service/>
          <service-group/>
          <schedule/>
          <rulebase/>
        </entry>
      </vsys>
    </entry>
  </devices>
</config>"#;

// ============================================================================
// Tests
// ============================================================================

#[test]
fn generated_config_022_panos_init_cfg() {
    let t = PaloAltoPanosZtpTemplate {
        hostname: "panos01".to_string(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
        mgmt_netmask: Ipv4Addr::new(255, 255, 255, 0),
        mgmt_gateway: Ipv4Addr::new(172, 20, 0, 1),
        dns_primary: Ipv4Addr::new(172, 20, 0, 1),
        dns_secondary: Ipv4Addr::new(8, 8, 8, 8),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_INIT_IPV4);
}

#[test]
fn generated_config_023_panos_bootstrap_xml() {
    let hash = "$5$saltsalt$HASHHASHHASHHASHHASHHASHHASHHASHHASHHASHHAS";
    let t = PaloAltoPanosBootstrapTemplate {
        hostname: "panos01".to_string(),
        owner: helpers::test_owner(hash),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
        mgmt_netmask: Ipv4Addr::new(255, 255, 255, 0),
        mgmt_gateway: Ipv4Addr::new(172, 20, 0, 1),
        dns_primary: Ipv4Addr::new(172, 20, 0, 1),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_BOOTSTRAP_IPV4.replace("HASH", hash));
}
