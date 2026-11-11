//! Which sidebar link a page highlights. The handler names its page; the sidebar is given each
//! link's state.

/// The page a handler renders, as the sidebar knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavPage {
    Dashboard,
    Labs,
    AdminImages,
    AdminLabs,
    AdminUsers,
    AdminTools,
    /// A page with no sidebar link, such as the profile or an error page.
    Other,
}

/// How the sidebar draws one link.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NavLink {
    /// The link to the page being shown, highlighted.
    Current,
    #[default]
    Inactive,
}

/// The sidebar's links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Nav {
    pub dashboard: NavLink,
    pub labs: NavLink,
    pub admin_images: NavLink,
    pub admin_labs: NavLink,
    pub admin_users: NavLink,
    pub admin_tools: NavLink,
}

impl From<NavPage> for Nav {
    fn from(page: NavPage) -> Self {
        let none = Self::default();
        match page {
            NavPage::Dashboard => Self {
                dashboard: NavLink::Current,
                ..none
            },
            NavPage::Labs => Self {
                labs: NavLink::Current,
                ..none
            },
            NavPage::AdminImages => Self {
                admin_images: NavLink::Current,
                ..none
            },
            NavPage::AdminLabs => Self {
                admin_labs: NavLink::Current,
                ..none
            },
            NavPage::AdminUsers => Self {
                admin_users: NavLink::Current,
                ..none
            },
            NavPage::AdminTools => Self {
                admin_tools: NavLink::Current,
                ..none
            },
            NavPage::Other => none,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The current links, by name.
    fn current(nav: Nav) -> Vec<&'static str> {
        [
            (nav.dashboard, "dashboard"),
            (nav.labs, "labs"),
            (nav.admin_images, "admin_images"),
            (nav.admin_labs, "admin_labs"),
            (nav.admin_users, "admin_users"),
            (nav.admin_tools, "admin_tools"),
        ]
        .into_iter()
        .filter_map(|(link, name)| (link == NavLink::Current).then_some(name))
        .collect()
    }

    #[test]
    fn each_page_highlights_its_own_link_alone() {
        for (page, links) in [
            (NavPage::Dashboard, vec!["dashboard"]),
            (NavPage::Labs, vec!["labs"]),
            (NavPage::AdminImages, vec!["admin_images"]),
            (NavPage::AdminLabs, vec!["admin_labs"]),
            (NavPage::AdminUsers, vec!["admin_users"]),
            (NavPage::AdminTools, vec!["admin_tools"]),
            (NavPage::Other, vec![]),
        ] {
            assert_eq!(current(Nav::from(page)), links, "{page:?}");
        }
    }
}
