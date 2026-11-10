use serde::{Deserialize, Serialize};

/// The caller of a request: the user and the login session it came through.
///
/// It is built from the session row (`auth::session::lookup`), so `username` and `is_admin` are
/// the user's current values, not those at login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthContext {
    /// Username of the authenticated user
    pub username: String,
    /// Whether the user is an admin
    pub is_admin: bool,
    /// The session the request came through: the one logout ends, and the one a user's own
    /// password change keeps.
    pub session_id: String,
}

impl AuthContext {
    /// Create a new `AuthContext`
    pub fn new(username: String, is_admin: bool, session_id: String) -> Self {
        Self {
            username,
            is_admin,
            session_id,
        }
    }

    /// Check if the user can access a resource owned by the specified username
    ///
    /// Returns true if:
    /// - The user is an admin (can access all resources), OR
    /// - The user owns the resource (username matches)
    pub fn can_access(&self, owner_username: &str) -> bool {
        self.is_admin || self.username == owner_username
    }

    /// Redeploy authorization is restricted to the current lab owner.
    #[tracing::instrument(level = "debug", skip(self))]
    pub fn can_redeploy(&self, owner_username: &str) -> bool {
        self.username == owner_username
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl AuthContext {
        /// A caller with a placeholder session, for policy tests that do not look at it.
        pub(crate) fn for_test(username: &str, is_admin: bool) -> Self {
            Self::new(username.to_owned(), is_admin, "test-session".to_owned())
        }
    }

    #[test]
    fn only_the_current_owner_can_redeploy_regardless_of_admin_role() {
        for is_admin in [false, true] {
            let ctx = AuthContext::for_test("alice", is_admin);
            assert!(ctx.can_redeploy("alice"));
            assert!(!ctx.can_redeploy("bob"));
        }
    }

    #[test]
    fn test_regular_user_can_access_own_resources() {
        let ctx = AuthContext::for_test("alice", false);
        assert!(ctx.can_access("alice"));
        assert!(!ctx.can_access("bob"));
    }

    #[test]
    fn test_admin_can_access_all_resources() {
        let ctx = AuthContext::for_test("admin", true);
        assert!(ctx.can_access("alice"));
        assert!(ctx.can_access("bob"));
        assert!(ctx.can_access("admin"));
    }

    #[test]
    fn test_user_cannot_access_others_resources() {
        let ctx = AuthContext::for_test("alice", false);
        assert!(!ctx.can_access("bob"));
        assert!(!ctx.can_access("charlie"));
    }
}
