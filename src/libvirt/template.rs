use askama::Template;

#[derive(Template)]
#[template(
    source = r#"
<domain>
    <name>{{ name }}</name>
    <vcpu>{{ cpus }}</vcpu>
    <memory unit="MB">{{ memory }}</memory>
</domain>"#,
    ext = "xml"
)]
pub struct DomainTemplate<'a> {
    pub name: &'a str,
    pub cpus: u8,
    pub memory: u16,
}
