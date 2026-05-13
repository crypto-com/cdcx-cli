use cdcx_core::config::McpConfig;
use cdcx_core::error::CdcxError;

const VALID_SERVICES: &[&str] = &[
    "market", "account", "trade", "advanced", "margin", "staking", "funding", "fiat", "otc",
    "stream",
];

fn validate_service(name: &str) -> Result<(), CdcxError> {
    if name != "all" && !VALID_SERVICES.contains(&name) {
        return Err(CdcxError::Config(format!(
            "Unknown service group: '{}'. Valid groups: {}, all",
            name,
            VALID_SERVICES.join(", ")
        )));
    }
    Ok(())
}

pub fn show_config() -> Result<(), CdcxError> {
    let config = McpConfig::load_default()?.unwrap_or_default();
    let path = McpConfig::default_path();
    let path_display = path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<unknown>".to_string());
    let file_exists = path.map(|p| p.exists()).unwrap_or(false);

    eprintln!("MCP configuration:");
    eprintln!(
        "  file:            {}{}",
        path_display,
        if file_exists { "" } else { " (using defaults)" }
    );
    eprintln!("  services:        {}", config.services.join(", "));
    eprintln!("  allow_dangerous: {}", config.allow_dangerous);
    Ok(())
}

pub fn enable_service(service: &str) -> Result<(), CdcxError> {
    validate_service(service)?;
    let mut config = McpConfig::load_default()?.unwrap_or_default();
    if service == "all" {
        config.services = VALID_SERVICES.iter().map(|s| s.to_string()).collect();
    } else if !config.services.iter().any(|s| s == service) {
        config.services.push(service.to_string());
    }
    config.save()?;
    eprintln!("Enabled service: {}", service);
    eprintln!("Active services: {}", config.services.join(", "));
    Ok(())
}

pub fn disable_service(service: &str) -> Result<(), CdcxError> {
    validate_service(service)?;
    let mut config = McpConfig::load_default()?.unwrap_or_default();
    if service == "all" {
        config.services = vec!["market".to_string()];
    } else {
        config.services.retain(|s| s != service);
        if config.services.is_empty() {
            config.services.push("market".to_string());
            eprintln!("Cannot disable all services; keeping 'market' as minimum.");
        }
    }
    config.save()?;
    eprintln!("Disabled service: {}", service);
    eprintln!("Active services: {}", config.services.join(", "));
    Ok(())
}

pub fn set_allow_dangerous(enabled: bool) -> Result<(), CdcxError> {
    let mut config = McpConfig::load_default()?.unwrap_or_default();
    config.allow_dangerous = enabled;
    config.save()?;
    if enabled {
        eprintln!("Dangerous operations: enabled");
    } else {
        eprintln!("Dangerous operations: disabled");
    }
    Ok(())
}

pub fn reset_config() -> Result<(), CdcxError> {
    McpConfig::delete()?;
    eprintln!("MCP configuration reset to defaults.");
    eprintln!("  services:        market");
    eprintln!("  allow_dangerous: false");
    Ok(())
}
