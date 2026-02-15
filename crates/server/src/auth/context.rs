use serde::{Deserialize, Serialize};

/// Authentication context extracted from a validated JWT token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthContext {
    /// Username of the authenticated user
    pub username: String,
    /// Whether the user is an admin
    pub is_admin: bool,
}

impl AuthContext {
    /// Create a new AuthContext
    pub fn new(username: String, is_admin: bool) -> Self {
        Self { username, is_admin }
    }

    /// Check if the user can access a resource owned by the specified username
    ///
    /// Returns true if:
    /// - The user is an admin (can access all resources), OR
    /// - The user owns the resource (username matches)
    pub fn can_access(&self, owner_username: &str) -> bool {
        self.is_admin || self.username == owner_username
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regular_user_can_access_own_resources() {
        let ctx = AuthContext::new("alice".to_string(), false);
        assert!(ctx.can_access("alice"));
        assert!(!ctx.can_access("bob"));
    }

    #[test]
    fn test_admin_can_access_all_resources() {
        let ctx = AuthContext::new("admin".to_string(), true);
        assert!(ctx.can_access("alice"));
        assert!(ctx.can_access("bob"));
        assert!(ctx.can_access("admin"));
    }

    #[test]
    fn test_user_cannot_access_others_resources() {
        let ctx = AuthContext::new("alice".to_string(), false);
        assert!(!ctx.can_access("bob"));
        assert!(!ctx.can_access("charlie"));
    }
}
