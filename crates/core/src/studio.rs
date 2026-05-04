//! Pylon Studio runtime configuration.
//!
//! This is the Rust mirror of `packages/sdk/src/studio.ts`. Authored by
//! the user as `studio.config.ts`, compiled to JSON by the CLI, read at
//! runtime, and injected into the Studio HTML as
//! `window.__PYLON_STUDIO_CONFIG__`.
//!
//! Every field is optional — an empty config is valid, and the web shell
//! falls back to a sensible default (manifest entities → resources,
//! emerald accent, Used Space footer card).
//!
//! Wire format is camelCase JSON to match the TS authoring surface.
//! `serde(default)` everywhere so partial configs round-trip cleanly.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Top-level
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StudioConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<BrandConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<SidebarConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resources: BTreeMap<String, ResourceConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pages: BTreeMap<String, PageConfig>,
    /// True when the project ships a `studio.entry.tsx` bundle. Studio
    /// HTML dynamic-imports `/studio/extensions.js` only when this flag
    /// is set.
    #[serde(default)]
    pub has_extensions: bool,
    /// URL to redirect unauthenticated `/studio` callers to. Apps with
    /// their own login page (Pylon Cloud, dashboards) point this at
    /// their existing flow so users don't see the built-in admin-token
    /// form. The framework appends `?next=/studio`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Brand
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrandConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThemeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<ThemeAccent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appearance: Option<ThemeAppearance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeAccent {
    Emerald,
    Blue,
    Violet,
    Rose,
    Amber,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeAppearance {
    Dark,
    Light,
    System,
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidebarConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<SidebarSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<SidebarFooter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_switcher: Option<OrgSwitcherConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsible: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidebarSection {
    pub label: String,
    #[serde(default)]
    pub items: Vec<SidebarItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_open: Option<bool>,
}

/// Discriminated union via `type` field. Matches the TS shape exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SidebarItem {
    Page(SidebarPageItem),
    Resource(SidebarResourceItem),
    Link(SidebarLinkItem),
    Heading(SidebarHeadingItem),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidebarPageItem {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_admin: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_roles: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidebarResourceItem {
    pub entity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_admin: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_roles: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidebarLinkItem {
    pub label: String,
    pub href: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidebarHeadingItem {
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SidebarFooter {
    Card(SidebarFooterCard),
    Custom(SidebarFooterCustom),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidebarFooterCard {
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<FooterAction>,
    /// 0..1 progress fill. Stored as an `f64` so the JSON round-trips
    /// the user's original number unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FooterAction {
    pub label: String,
    pub href: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidebarFooterCustom {
    pub component_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrgSwitcherConfig {
    pub items: Vec<OrgSwitcherItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrgSwitcherItem {
    pub id: String,
    pub label: String,
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResourceConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plural_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<ResourceListConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResourceListConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<ColumnConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub searchable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filterable: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bulk_actions: Vec<BulkAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_actions: Vec<RowAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_sort: Option<DefaultSort>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub page_sizes: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_page_size: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DefaultSort {
    pub field: String,
    pub order: SortOrder,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColumnConfig {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sortable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub searchable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filterable: Option<ColumnFilterable>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renderer: Option<ColumnRenderer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<ColumnAlign>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ColumnFilterable {
    Bool(bool),
    Spec(ColumnFilterSpec),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColumnFilterSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<FilterOption>,
}

/// Filter option value is preserved as a `serde_json::Value` so any
/// JSON shape (string, number, bool, null, object) round-trips intact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterOption {
    pub label: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnAlign {
    Left,
    Center,
    Right,
}

// ---------------------------------------------------------------------------
// Renderers — discriminated union on `kind`
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ColumnRenderer {
    Text(RendererText),
    Avatar(RendererAvatar),
    Badge(RendererBadge),
    Date(RendererDate),
    Link(RendererLink),
    Boolean(RendererBoolean),
    Number(RendererNumber),
    Json(RendererJson),
    Custom(RendererCustom),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RendererText {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncate: Option<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub mono: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RendererAvatar {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_field: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RendererBadge {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variants: BTreeMap<String, BadgeVariant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dot: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BadgeVariant {
    Green,
    Red,
    Amber,
    Blue,
    Gray,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RendererDate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<DateFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DateFormat {
    Relative,
    Absolute,
    Iso,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RendererLink {
    pub href: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RendererBoolean {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub true_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub false_label: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RendererNumber {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<NumberStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NumberStyle {
    Decimal,
    Percent,
    Currency,
    Bytes,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RendererJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncate: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RendererCustom {
    pub component_id: String,
}

// ---------------------------------------------------------------------------
// Bulk + row actions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BulkAction {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<BulkActionKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_admin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BulkActionKind {
    Delete,
    Export,
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RowAction {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<RowActionKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_admin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowActionKind {
    Delete,
    Edit,
    View,
    Custom,
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PageConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_id: Option<String>,
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn is_false(b: &bool) -> bool {
    !*b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_round_trips() {
        let cfg = StudioConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: StudioConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn parse_minimal_user_config() {
        let json = r#"{
            "brand": { "name": "Acme" },
            "theme": { "accent": "emerald", "appearance": "dark" },
            "sidebar": {
                "sections": [{
                    "label": "RESOURCES",
                    "items": [
                        { "type": "resource", "entity": "User", "icon": "users" },
                        { "type": "page", "id": "overview", "label": "Overview" }
                    ]
                }]
            },
            "resources": {
                "User": {
                    "list": {
                        "columns": [
                            {
                                "field": "status",
                                "renderer": {
                                    "kind": "badge",
                                    "variants": { "active": "green", "blocked": "red" }
                                }
                            }
                        ]
                    }
                }
            }
        }"#;
        let cfg: StudioConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.brand.unwrap().name.unwrap(), "Acme");
        assert_eq!(cfg.theme.unwrap().accent.unwrap(), ThemeAccent::Emerald);
        assert_eq!(cfg.sidebar.unwrap().sections.len(), 1);
        let user = cfg.resources.get("User").unwrap();
        let col = &user.list.as_ref().unwrap().columns[0];
        match col.renderer.as_ref().unwrap() {
            ColumnRenderer::Badge(b) => {
                assert_eq!(b.variants.get("active"), Some(&BadgeVariant::Green));
                assert_eq!(b.variants.get("blocked"), Some(&BadgeVariant::Red));
            }
            _ => panic!("expected Badge renderer"),
        }
    }

    #[test]
    fn parse_link_and_heading_items() {
        let json = r#"{
            "sidebar": {
                "sections": [{
                    "label": "ACCOUNTS",
                    "items": [
                        { "type": "heading", "label": "External" },
                        { "type": "link", "label": "Google Analytics", "href": "https://analytics.google.com" }
                    ]
                }]
            }
        }"#;
        let cfg: StudioConfig = serde_json::from_str(json).unwrap();
        let items = &cfg.sidebar.unwrap().sections[0].items;
        assert!(matches!(items[0], SidebarItem::Heading(_)));
        match &items[1] {
            SidebarItem::Link(l) => {
                assert_eq!(l.label, "Google Analytics");
                assert_eq!(l.href, "https://analytics.google.com");
            }
            _ => panic!("expected Link"),
        }
    }
}
