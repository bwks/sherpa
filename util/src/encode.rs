use anyhow::Result;

use base64::{engine::general_purpose, Engine};

use crate::util::load_file;

pub fn base64_encode(input: &str) -> String {
    general_purpose::STANDARD.encode(input)
}

pub fn base64_encode_file(filename: &str) -> Result<String> {
    let file_content = load_file(filename)?;
    Ok(base64_encode(&file_content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_encode_empty_string() {
        assert_eq!(base64_encode(""), "");
    }

    #[test]
    fn test_base64_encode_simple_string() {
        assert_eq!(base64_encode("Hello, World!"), "SGVsbG8sIFdvcmxkIQ==");
    }

    #[test]
    fn test_base64_encode_with_special_characters() {
        assert_eq!(
            base64_encode("Special@#$%^&*()"),
            "U3BlY2lhbEAjJCVeJiooKQ=="
        );
    }

    #[test]
    fn test_base64_encode_unicode() {
        assert_eq!(base64_encode("こんにちは"), "44GT44KT44Gr44Gh44Gv");
    }

    #[test]
    fn test_base64_encode_long_text() {
        assert_eq!(
            base64_encode(
                r#"!
hostname iosv-ztp
ip domain name sherpa.lab.local
no ip domain lookup
crypto key generate rsa modulus 2048
ip ssh version 2
!
aaa new-model
aaa authentication login default local
aaa authorization exec default local
!
username sherpa privilege 15 secret Gang!42069
!
ip ssh pubkey-chain
  username sherpa
   key-hash ssh-ed25519 102313C1E00EE89C37D6E98DB3513E5B
!
!
interface GigabitEthernet0/0
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
 action 2 cli command "interface GigabitEthernet0/0"
 action 3 cli command "no shutdown"
 action 4 cli command "exit"
 action 5 cli command "crypto key generate rsa modulus 2048"
!"#
            ),
            "IQpob3N0bmFtZSBpb3N2LXp0cAppcCBkb21haW4gbmFtZSBzaGVycGEubGFiLmxvY2FsCm5vIGlwIGRvbWFpbiBsb29rdXAKY3J5cHRvIGtleSBnZW5lcmF0ZSByc2EgbW9kdWx1cyAyMDQ4CmlwIHNzaCB2ZXJzaW9uIDIKIQphYWEgbmV3LW1vZGVsCmFhYSBhdXRoZW50aWNhdGlvbiBsb2dpbiBkZWZhdWx0IGxvY2FsCmFhYSBhdXRob3JpemF0aW9uIGV4ZWMgZGVmYXVsdCBsb2NhbAohCnVzZXJuYW1lIHNoZXJwYSBwcml2aWxlZ2UgMTUgc2VjcmV0IEdhbmchNDIwNjkKIQppcCBzc2ggcHVia2V5LWNoYWluCiAgdXNlcm5hbWUgc2hlcnBhCiAgIGtleS1oYXNoIHNzaC1lZDI1NTE5IDEwMjMxM0MxRTAwRUU4OUMzN0Q2RTk4REIzNTEzRTVCCiEKIQppbnRlcmZhY2UgR2lnYWJpdEV0aGVybmV0MC8wCiBpcCBhZGRyZXNzIGRoY3AKIG5lZ290aWF0aW9uIGF1dG8KIG5vIHNodXRkb3duCiBleGl0CiEKbGluZSBjb24gMAogbG9nZ2luZyBzeW5jaHJvbm91cwogc3RvcGJpdHMgMQogZXhpdAohCmxpbmUgdnR5IDAgNAogbG9nZ2luZyBzeW5jaHJvbm91cwogdHJhbnNwb3J0IGlucHV0IHNzaAogZXhpdAohCmV2ZW50IG1hbmFnZXIgYXBwbGV0IEVOQUJMRS1NR01UCiBldmVudCBzeXNsb2cgcGF0dGVybiAiU1lTLTUtUkVTVEFSVCIKIGFjdGlvbiAwIGNsaSBjb21tYW5kICJlbmFibGUiCiBhY3Rpb24gMSBjbGkgY29tbWFuZCAiY29uZiB0IgogYWN0aW9uIDIgY2xpIGNvbW1hbmQgImludGVyZmFjZSBHaWdhYml0RXRoZXJuZXQwLzAiCiBhY3Rpb24gMyBjbGkgY29tbWFuZCAibm8gc2h1dGRvd24iCiBhY3Rpb24gNCBjbGkgY29tbWFuZCAiZXhpdCIKIGFjdGlvbiA1IGNsaSBjb21tYW5kICJjcnlwdG8ga2V5IGdlbmVyYXRlIHJzYSBtb2R1bHVzIDIwNDgiCiE="
        );
    }
}
