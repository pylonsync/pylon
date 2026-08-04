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
    /// URL to redirect unauthenticated `/studio` callers to. Studio has no
    /// login page of its own — it authenticates whoever the app already
    /// signed in — so apps point this at their existing flow. The framework
    /// appends `?next=/studio`. Unset, anonymous callers get a static page
    /// explaining how to designate an admin.
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
    /// Where the control renders. Defaults to the trailing `…` menu.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<RowActionDisplay>,
    /// Function name for `kind: "action"`, POSTed to `/api/fn/<action>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Argument object for `kind: "action"`. String values interpolate
    /// `{row.<field>}`; a value that is exactly one placeholder keeps the
    /// row value's JSON type. Defaults to `{ "id": <row id> }`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Map<String, serde_json::Value>>,
    /// What Studio does with the function's return value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<RowActionResult>,
    /// Dot path into the return value, for `result: "copy"` / `"toast"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_field: Option<String>,
    /// Reload the table after the action succeeds. Defaults to true —
    /// an action that touched the row should not leave a stale display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<bool>,
    /// Button styling for `display: "button"`. Defaults to `outline`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<RowActionVariant>,
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
    /// Call a server function. See [`RowAction::action`].
    Action,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowActionDisplay {
    Menu,
    Button,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowActionResult {
    Toast,
    Copy,
    Dialog,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowActionVariant {
    Default,
    Outline,
    Ghost,
    Secondary,
    Destructive,
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

    /// The whole point of this struct is that it's the *only* path a
    /// `studio.config.ts` takes into the browser: the CLI parses the
    /// bun-emitted JSON into `StudioConfig` and re-serializes it. A field
    /// the SDK declares but this file doesn't is dropped on the floor —
    /// it typechecks, it ships, and it does nothing. So parse a full
    /// action row-button config and assert every field survives.
    #[test]
    fn row_action_button_round_trips() {
        let json = r#"{
            "resources": {
                "Proposal": {
                    "list": {
                        "rowActions": [
                            {
                                "id": "generateLink",
                                "label": "Generate link",
                                "icon": "link",
                                "kind": "action",
                                "display": "button",
                                "action": "generateProposalLink",
                                "input": { "proposalId": "{row.id}", "expiresDays": 30 },
                                "result": "copy",
                                "resultField": "url",
                                "refresh": false,
                                "variant": "outline",
                                "confirm": "Mint a new public link?",
                                "requiresAdmin": true
                            }
                        ]
                    }
                }
            }
        }"#;
        let cfg: StudioConfig = serde_json::from_str(json).unwrap();
        let a = &cfg.resources["Proposal"].list.as_ref().unwrap().row_actions[0];
        assert_eq!(a.id, "generateLink");
        assert_eq!(a.icon.as_deref(), Some("link"));
        assert_eq!(a.kind, Some(RowActionKind::Action));
        assert_eq!(a.display, Some(RowActionDisplay::Button));
        assert_eq!(a.action.as_deref(), Some("generateProposalLink"));
        let input = a.input.as_ref().unwrap();
        assert_eq!(input["proposalId"], serde_json::json!("{row.id}"));
        assert_eq!(input["expiresDays"], serde_json::json!(30));
        assert_eq!(a.result, Some(RowActionResult::Copy));
        assert_eq!(a.result_field.as_deref(), Some("url"));
        assert_eq!(a.refresh, Some(false));
        assert_eq!(a.variant, Some(RowActionVariant::Outline));
        assert_eq!(a.confirm.as_deref(), Some("Mint a new public link?"));
        assert!(a.requires_admin);

        // Re-serialize and read it back: this is the leg the browser
        // actually receives.
        let back: StudioConfig =
            serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back, cfg);
    }

    /// A row action with nothing but the two required fields is valid —
    /// the client falls back to the `…` menu and `{ id: <row id> }`.
    #[test]
    fn minimal_row_action_parses() {
        let json = r#"{"resources":{"X":{"list":{"rowActions":[{"id":"ping","label":"Ping"}]}}}}"#;
        let cfg: StudioConfig = serde_json::from_str(json).unwrap();
        let a = &cfg.resources["X"].list.as_ref().unwrap().row_actions[0];
        assert_eq!(a.kind, None);
        assert_eq!(a.display, None);
        assert_eq!(a.input, None);
        assert!(!a.requires_admin);
    }

    /// Unknown enum values fail loudly at build time (the CLI surfaces a
    /// STUDIO_CONFIG_PARSE diagnostic) rather than silently disabling the
    /// action at runtime.
    #[test]
    fn unknown_row_action_kind_is_a_parse_error() {
        let json = r#"{"resources":{"X":{"list":{"rowActions":[{"id":"a","label":"A","kind":"teleport"}]}}}}"#;
        assert!(serde_json::from_str::<StudioConfig>(json).is_err());
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
