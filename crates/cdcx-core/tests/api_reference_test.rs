//! Validates schema registry parsing and endpoint structure
//! using the test-spec.yaml fixture.

use cdcx_core::schema::SchemaRegistry;

#[test]
fn all_fixture_endpoints_have_valid_methods() {
    let registry = SchemaRegistry::from_fixture().expect("fixture spec should parse");

    for ep in registry.list_all() {
        assert!(
            ep.method.starts_with("public/") || ep.method.starts_with("private/"),
            "Endpoint method should start with public/ or private/, got: {}",
            ep.method
        );
    }
}

#[test]
fn fixture_covers_core_groups() {
    let registry = SchemaRegistry::from_fixture().expect("fixture spec should parse");
    let groups = registry.groups();

    assert!(groups.contains(&"market"), "missing market group");
    assert!(groups.contains(&"trade"), "missing trade group");
    assert!(groups.contains(&"account"), "missing account group");
    assert!(
        groups.len() >= 3,
        "Expected at least 3 groups, got {}",
        groups.len()
    );
}
