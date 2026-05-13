use cdcx_core::config::{valid_mcp_service_names, McpConfig};
use cdcx_core::error::CdcxError;

fn validate_service(name: &str) -> Result<(), CdcxError> {
    let valid = valid_mcp_service_names();
    if name != "all" && !valid.contains(&name) {
        return Err(CdcxError::Config(format!(
            "Unknown service group: '{}'. Valid groups: {}, all",
            name,
            valid.join(", ")
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

    println!("MCP configuration:");
    println!(
        "  file:            {}{}",
        path_display,
        if file_exists { "" } else { " (using defaults)" }
    );
    println!("  services:        {}", config.services.join(", "));
    println!("  allow_dangerous: {}", config.allow_dangerous);
    Ok(())
}

pub fn enable_service(input: &str) -> Result<(), CdcxError> {
    let names: Vec<&str> = input.split(',').map(|s| s.trim()).collect();
    for name in &names {
        validate_service(name)?;
    }
    let mut config = McpConfig::load_default()?.unwrap_or_default();
    let valid = valid_mcp_service_names();
    for name in &names {
        if *name == "all" {
            config.services = valid.iter().map(|s| s.to_string()).collect();
            break;
        } else if !config.services.iter().any(|s| s == name) {
            config.services.push(name.to_string());
        }
    }
    config.save()?;
    eprintln!("Active services: {}", config.services.join(", "));
    Ok(())
}

pub fn disable_service(input: &str) -> Result<(), CdcxError> {
    let names: Vec<&str> = input.split(',').map(|s| s.trim()).collect();
    for name in &names {
        validate_service(name)?;
    }
    let mut config = McpConfig::load_default()?.unwrap_or_default();
    for name in &names {
        if *name == "all" {
            config.services = vec!["market".to_string()];
            break;
        } else {
            config.services.retain(|s| s != name);
        }
    }
    if config.services.is_empty() {
        config.services.push("market".to_string());
        eprintln!("Cannot disable all services; keeping 'market' as minimum.");
    }
    config.save()?;
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
