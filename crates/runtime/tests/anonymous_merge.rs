//! End-to-end coverage for `pylon_router::merge::transfer_user_ownership`
//! against a real `Runtime` (in-memory SQLite + manifest with cross-entity
//! `id(User)` references). Companion to the unit tests in
//! `crates/router/src/merge.rs::tests` which only cover the pure
//! field-discovery logic.
//!
//! Each test stands up a tiny manifest with `User`, `Cart`, and `Order`
//! entities and inserts seed rows owned by a guest user. The merge then
//! has to rewrite the owner column on every entity that actually
//! references `User`. Asserting against a real store catches integration
//! bugs the unit tests can't (e.g. wrong filter shape, accidentally
//! triggering CRDT writes that drop the field).

use pylon_http::DataStore;
use pylon_kernel::*;
use pylon_router::merge::{find_user_referencing_fields, transfer_user_ownership};
use serde_json::json;

fn field(name: &str, ty: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: ty.into(),
        optional: false,
        unique: false,
        crdt: None,
    }
}

fn entity(name: &str, fields: Vec<ManifestField>) -> ManifestEntity {
    ManifestEntity {
        name: name.into(),
        fields,
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: true,
    }
}

fn merge_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "merge-test".into(),
        version: "0.1.0".into(),
        entities: vec![
            entity(
                "User",
                vec![field("email", "string"), field("displayName", "string")],
            ),
            entity(
                "Cart",
                vec![field("userId", "id(User)"), field("note", "string")],
            ),
            entity(
                "Order",
                vec![field("buyerId", "id(User)"), field("amount", "int")],
            ),
            // Decoy: same field NAME but different referenced entity. Must
            // not be touched by the merge.
            entity("Doc", vec![field("ownerId", "id(Project)")]),
        ],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
    }
}

#[test]
fn transfer_moves_rows_across_entities_and_skips_decoys() {
    let runtime = pylon_runtime::Runtime::in_memory(merge_manifest()).unwrap();
    let store: &dyn DataStore = &runtime;

    let guest_id = store
        .insert(
            "User",
            &json!({ "email": "guest@x", "displayName": "Guest" }),
        )
        .unwrap();
    let real_id = store
        .insert("User", &json!({ "email": "real@x", "displayName": "Real" }))
        .unwrap();

    // Guest accumulates state.
    let cart_a = store
        .insert("Cart", &json!({ "userId": &guest_id, "note": "thing 1" }))
        .unwrap();
    let cart_b = store
        .insert("Cart", &json!({ "userId": &guest_id, "note": "thing 2" }))
        .unwrap();
    // Cart owned by someone else — must NOT be touched.
    let cart_other_owner = "user-other".to_string();
    let cart_c = store
        .insert(
            "Cart",
            &json!({ "userId": &cart_other_owner, "note": "someone else" }),
        )
        .unwrap();
    let order_a = store
        .insert("Order", &json!({ "buyerId": &guest_id, "amount": 99 }))
        .unwrap();
    // Doc references id(Project), not id(User). Must NOT be touched even
    // though `ownerId == guest_id` happens to match by string.
    let doc = store
        .insert("Doc", &json!({ "ownerId": &guest_id }))
        .unwrap();

    let result = transfer_user_ownership(store, store.manifest(), &guest_id, &real_id, "User");
    assert_eq!(result.rows_updated, 3, "2 carts + 1 order should move");
    // Touched entries are sorted by manifest order: Cart, Order.
    assert_eq!(
        result.touched,
        vec![
            ("Cart".to_string(), "userId".to_string(), 2),
            ("Order".to_string(), "buyerId".to_string(), 1),
        ]
    );

    // Guest carts now belong to the real user.
    let a = store.get_by_id("Cart", &cart_a).unwrap().unwrap();
    let b = store.get_by_id("Cart", &cart_b).unwrap().unwrap();
    assert_eq!(a["userId"], real_id);
    assert_eq!(b["userId"], real_id);

    // Other user's cart untouched.
    let c = store.get_by_id("Cart", &cart_c).unwrap().unwrap();
    assert_eq!(c["userId"], cart_other_owner);

    // Order moved.
    let o = store.get_by_id("Order", &order_a).unwrap().unwrap();
    assert_eq!(o["buyerId"], real_id);

    // Decoy Doc untouched — its field references Project, not User.
    let d = store.get_by_id("Doc", &doc).unwrap().unwrap();
    assert_eq!(d["ownerId"], guest_id);
}

#[test]
fn self_merge_is_a_noop() {
    let runtime = pylon_runtime::Runtime::in_memory(merge_manifest()).unwrap();
    let store: &dyn DataStore = &runtime;
    let id = store
        .insert("User", &json!({ "email": "a@x", "displayName": "A" }))
        .unwrap();
    store
        .insert("Cart", &json!({ "userId": &id, "note": "n" }))
        .unwrap();
    let result = transfer_user_ownership(store, store.manifest(), &id, &id, "User");
    assert_eq!(result.rows_updated, 0);
    assert!(result.touched.is_empty());
}

#[test]
fn empty_merge_returns_no_touched_entries() {
    let runtime = pylon_runtime::Runtime::in_memory(merge_manifest()).unwrap();
    let store: &dyn DataStore = &runtime;
    // Guest has no rows anywhere.
    let result = transfer_user_ownership(store, store.manifest(), "ghost-guest", "real-id", "User");
    assert_eq!(result.rows_updated, 0);
    assert!(result.touched.is_empty());
    assert_eq!(result.entities_csv(), "");
}

#[test]
fn entities_csv_format() {
    let runtime = pylon_runtime::Runtime::in_memory(merge_manifest()).unwrap();
    let store: &dyn DataStore = &runtime;
    let guest = store
        .insert("User", &json!({ "email": "g@x", "displayName": "G" }))
        .unwrap();
    let real = store
        .insert("User", &json!({ "email": "r@x", "displayName": "R" }))
        .unwrap();
    store
        .insert("Cart", &json!({ "userId": &guest, "note": "c1" }))
        .unwrap();
    store
        .insert("Cart", &json!({ "userId": &guest, "note": "c2" }))
        .unwrap();
    store
        .insert("Order", &json!({ "buyerId": &guest, "amount": 5 }))
        .unwrap();
    let result = transfer_user_ownership(store, store.manifest(), &guest, &real, "User");
    assert_eq!(result.entities_csv(), "Cart.userId:2,Order.buyerId:1");
}

#[test]
fn find_user_referencing_fields_matches_runtime_manifest() {
    // Sanity: the field-discovery helper agrees with what the merge will
    // actually rewrite.
    let m = merge_manifest();
    let mut found = find_user_referencing_fields(&m, "User");
    found.sort();
    assert_eq!(
        found,
        vec![
            ("Cart".to_string(), "userId".to_string()),
            ("Order".to_string(), "buyerId".to_string()),
        ]
    );
}
