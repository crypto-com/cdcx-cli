//! Integration test: parse the real OpenAPI spec and verify endpoint extraction.
//! Fetches from remote if no cache exists.

use cdcx_core::openapi::fetcher::SpecFetcher;
use cdcx_core::schema::SchemaRegistry;

#[tokio::test]
async fn test_parse_real_openapi_spec() {
    let fetcher = SpecFetcher::default();

    let spec = match fetcher.load_cache() {
        Some(cached) => cached,
        None => match fetcher.fetch_remote().await {
            Ok(spec) => {
                let _ = fetcher.write_cache(&spec);
                spec
            }
            Err(e) => {
                eprintln!("Skipping integration test (no network): {}", e);
                return;
            }
        },
    };

    let registry = SchemaRegistry::from_openapi(&spec).expect("Failed to parse OpenAPI spec");

    // Should have at least 60 endpoints (75 expected)
    let all = registry.list_all();
    assert!(
        all.len() >= 60,
        "Expected at least 60 endpoints, got {}",
        all.len()
    );

    // Market group should have public endpoints
    let market = registry.get_by_group("market");
    assert!(
        market.len() >= 5,
        "Expected at least 5 market endpoints, got {}",
        market.len()
    );

    // Verify a known public endpoint
    let instruments = registry
        .get_by_method("public/get-instruments")
        .expect("public/get-instruments should exist");
    assert_eq!(instruments.command, "instruments");
    assert_eq!(instruments.safety_tier, "read");
    assert!(!instruments.auth_required);

    // Verify a known private endpoint
    let create_order = registry
        .get_by_method("private/create-order")
        .expect("private/create-order should exist");
    assert!(create_order.auth_required);
    assert_eq!(create_order.safety_tier, "mutate");

    // Verify groups are present
    let groups = registry.groups();
    assert!(groups.contains(&"market"), "Missing market group");
    assert!(groups.contains(&"trade"), "Missing trade group");
    assert!(groups.contains(&"account"), "Missing account group");
    assert!(groups.contains(&"wallet"), "Missing wallet group");

    eprintln!(
        "Integration test passed: {} endpoints across {} groups",
        all.len(),
        groups.len()
    );
}
