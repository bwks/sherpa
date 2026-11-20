//! Each device template with the lab owner as the one login: the owner's keys in each device's
//! own format, and no other account but the device's fixed vendor ones.
use askama::Template;

use template::WindowsSshdConfigTemplate;

#[test]
fn owner_login_025_windows_sshd_config_reads_each_accounts_own_keys_file() {
    let output = WindowsSshdConfigTemplate {
        authorized_keys_dir: "__PROGRAMDATA__/ssh/authorized_keys".to_string(),
        ready_marker: "sherpa-ready".to_string(),
    }
    .render()
    .unwrap();
    assert!(output.contains("AuthorizedKeysFile __PROGRAMDATA__/ssh/authorized_keys/%u\n"));
    assert!(!output.contains("Match Group administrators"));
}
