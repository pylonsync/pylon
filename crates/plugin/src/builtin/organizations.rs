use std::collections::HashMap;
use std::sync::Mutex;

use crate::Plugin;

/// Member role within an organization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrgRole {
    Owner,
    Admin,
    Member,
}

impl OrgRole {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }

    pub fn can_manage_members(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    pub fn can_delete_org(&self) -> bool {
        matches!(self, Self::Owner)
    }
}

/// An organization.
#[derive(Debug, Clone)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub created_by: String,
    pub created_at: String,
}

/// A membership in an organization.
#[derive(Debug, Clone)]
pub struct Membership {
    pub org_id: String,
    pub user_id: String,
    pub role: OrgRole,
    pub joined_at: String,
}

/// Organizations plugin. Multi-tenant team management with roles.
pub struct OrganizationsPlugin {
    orgs: Mutex<HashMap<String, Organization>>,
    members: Mutex<Vec<Membership>>,
    next_id: Mutex<u64>,
}

impl OrganizationsPlugin {
    pub fn new() -> Self {
        Self {
            orgs: Mutex::new(HashMap::new()),
            members: Mutex::new(Vec::new()),
            next_id: Mutex::new(0),
        }
    }

    /// Create a new organization. The creator becomes the owner.
    pub fn create_org(&self, name: &str, creator_id: &str) -> Organization {
        let mut id_counter = self.next_id.lock().unwrap();
        *id_counter += 1;
        let id = format!("org_{}", *id_counter);

        let org = Organization {
            id: id.clone(),
            name: name.to_string(),
            created_by: creator_id.to_string(),
            created_at: now(),
        };

        self.orgs.lock().unwrap().insert(id.clone(), org.clone());

        // Add creator as owner.
        self.members.lock().unwrap().push(Membership {
            org_id: id,
            user_id: creator_id.to_string(),
            role: OrgRole::Owner,
            joined_at: now(),
        });

        org
    }

    /// Add a member to an organization.
    pub fn add_member(&self, org_id: &str, user_id: &str, role: OrgRole) -> Result<(), String> {
        if !self.orgs.lock().unwrap().contains_key(org_id) {
            return Err(format!("Organization {} not found", org_id));
        }

        let mut members = self.members.lock().unwrap();
        if members.iter().any(|m| m.org_id == org_id && m.user_id == user_id) {
            return Err("User is already a member".into());
        }

        members.push(Membership {
            org_id: org_id.to_string(),
            user_id: user_id.to_string(),
            role,
            joined_at: now(),
        });
        Ok(())
    }

    /// Remove a member from an organization.
    pub fn remove_member(&self, org_id: &str, user_id: &str) -> bool {
        let mut members = self.members.lock().unwrap();
        let before = members.len();
        members.retain(|m| !(m.org_id == org_id && m.user_id == user_id));
        members.len() < before
    }

    /// Get a user's role in an organization.
    pub fn get_role(&self, org_id: &str, user_id: &str) -> Option<OrgRole> {
        self.members
            .lock()
            .unwrap()
            .iter()
            .find(|m| m.org_id == org_id && m.user_id == user_id)
            .map(|m| m.role.clone())
    }

    /// List all members of an organization.
    pub fn list_members(&self, org_id: &str) -> Vec<Membership> {
        self.members
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.org_id == org_id)
            .cloned()
            .collect()
    }

    /// List all organizations a user belongs to.
    pub fn list_user_orgs(&self, user_id: &str) -> Vec<(Organization, OrgRole)> {
        let members = self.members.lock().unwrap();
        let orgs = self.orgs.lock().unwrap();

        members
            .iter()
            .filter(|m| m.user_id == user_id)
            .filter_map(|m| {
                orgs.get(&m.org_id).map(|o| (o.clone(), m.role.clone()))
            })
            .collect()
    }

    /// Check if a user is a member of an organization.
    pub fn is_member(&self, org_id: &str, user_id: &str) -> bool {
        self.get_role(org_id, user_id).is_some()
    }

    /// Delete an organization and all its memberships.
    pub fn delete_org(&self, org_id: &str) -> bool {
        let removed = self.orgs.lock().unwrap().remove(org_id).is_some();
        if removed {
            self.members.lock().unwrap().retain(|m| m.org_id != org_id);
        }
        removed
    }
}

impl Plugin for OrganizationsPlugin {
    fn name(&self) -> &str {
        "organizations"
    }
}

fn now() -> String {
    let ts = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{ts}Z")
}

use std::time::SystemTime;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_org() {
        let plugin = OrganizationsPlugin::new();
        let org = plugin.create_org("My Team", "user-1");
        assert!(!org.id.is_empty());
        assert_eq!(org.name, "My Team");
        assert_eq!(org.created_by, "user-1");
    }

    #[test]
    fn creator_is_owner() {
        let plugin = OrganizationsPlugin::new();
        let org = plugin.create_org("Team", "user-1");
        let role = plugin.get_role(&org.id, "user-1").unwrap();
        assert_eq!(role, OrgRole::Owner);
    }

    #[test]
    fn add_and_list_members() {
        let plugin = OrganizationsPlugin::new();
        let org = plugin.create_org("Team", "user-1");
        plugin.add_member(&org.id, "user-2", OrgRole::Admin).unwrap();
        plugin.add_member(&org.id, "user-3", OrgRole::Member).unwrap();

        let members = plugin.list_members(&org.id);
        assert_eq!(members.len(), 3);
    }

    #[test]
    fn duplicate_member_rejected() {
        let plugin = OrganizationsPlugin::new();
        let org = plugin.create_org("Team", "user-1");
        let result = plugin.add_member(&org.id, "user-1", OrgRole::Member);
        assert!(result.is_err());
    }

    #[test]
    fn remove_member() {
        let plugin = OrganizationsPlugin::new();
        let org = plugin.create_org("Team", "user-1");
        plugin.add_member(&org.id, "user-2", OrgRole::Member).unwrap();

        assert!(plugin.remove_member(&org.id, "user-2"));
        assert!(!plugin.is_member(&org.id, "user-2"));
    }

    #[test]
    fn list_user_orgs() {
        let plugin = OrganizationsPlugin::new();
        let org1 = plugin.create_org("Team A", "user-1");
        let org2 = plugin.create_org("Team B", "user-2");
        plugin.add_member(&org2.id, "user-1", OrgRole::Member).unwrap();

        let orgs = plugin.list_user_orgs("user-1");
        assert_eq!(orgs.len(), 2);
    }

    #[test]
    fn role_permissions() {
        assert!(OrgRole::Owner.can_manage_members());
        assert!(OrgRole::Owner.can_delete_org());
        assert!(OrgRole::Admin.can_manage_members());
        assert!(!OrgRole::Admin.can_delete_org());
        assert!(!OrgRole::Member.can_manage_members());
        assert!(!OrgRole::Member.can_delete_org());
    }

    #[test]
    fn delete_org() {
        let plugin = OrganizationsPlugin::new();
        let org = plugin.create_org("Team", "user-1");
        plugin.add_member(&org.id, "user-2", OrgRole::Member).unwrap();

        assert!(plugin.delete_org(&org.id));
        assert!(plugin.list_members(&org.id).is_empty());
        assert!(!plugin.is_member(&org.id, "user-1"));
    }

    #[test]
    fn add_to_nonexistent_org() {
        let plugin = OrganizationsPlugin::new();
        let result = plugin.add_member("org_999", "user-1", OrgRole::Member);
        assert!(result.is_err());
    }
}
