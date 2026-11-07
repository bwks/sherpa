//! Which host resources belong to a lab, by name.
//!
//! Everything Sherpa creates for a lab is named `<name>-<lab_id>`: node domains and
//! containers, Docker and libvirt networks, bridges, taps and veths. Names are matched whole: a
//! substring match would take another lab's resources when one of its names contains this
//! lab's ID, and destroy deletes what matches.

/// Whether `name` is `<name>-<lab_id>`, a resource of the lab `lab_id`.
#[must_use]
pub fn is_lab_resource(name: &str, lab_id: &str) -> bool {
    name.strip_suffix(lab_id)
        .and_then(|rest| rest.strip_suffix('-'))
        .is_some_and(|prefix| !prefix.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAB_ID: &str = "fecb1a12";

    #[test]
    fn lab_resources_are_a_name_and_the_lab_id() {
        assert!(is_lab_resource("dev-fecb1a12", LAB_ID));
        assert!(is_lab_resource("web-1-fecb1a12", LAB_ID));
        assert!(is_lab_resource("tpa3-fecb1a12", LAB_ID));
        assert!(!is_lab_resource("fecb1a12-0a1b2c3d", LAB_ID));
        assert!(!is_lab_resource("fecb1a12", LAB_ID));
        assert!(!is_lab_resource("-fecb1a12", LAB_ID));
        assert!(!is_lab_resource("dev-fecb1a12x", LAB_ID));
    }
}
