//! Regression coverage for RUSTSEC-2026-0592.

#[test]
fn nested_blob_origins_are_opaque() {
    // Check shallow nesting first so an old dependency fails without aborting.
    for depth in [2, 10_000] {
        let input = format!("{}https://example.test/", "blob:".repeat(depth));
        let parsed = url::Url::parse(&input).unwrap();
        assert!(
            matches!(parsed.origin(), url::Origin::Opaque(_)),
            "nested blob origin at depth {depth}"
        );
        assert_eq!(pylon_auth::http_origin_of(&input), None);
    }
}

#[test]
fn http_origins_keep_scheme_host_and_port() {
    assert_eq!(
        pylon_auth::http_origin_of("https://example.test:8443/path?q=1"),
        Some("https://example.test:8443".into())
    );
    assert_eq!(
        url::Url::parse("blob:https://example.test/id")
            .unwrap()
            .origin()
            .ascii_serialization(),
        "https://example.test"
    );
}
