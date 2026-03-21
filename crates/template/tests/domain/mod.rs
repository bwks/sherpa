// The libvirt domain template (libvirt_domain.jinja) generates 237 lines of XML
// with many fields from shared::data types (Interface, NodeDisk, QemuCommand, etc.)
// that require complex setup. Tests for domain XML rendering should be added when
// the domain builder (server::services::node_ops::build_domain_template) is tested
// at the integration level.
