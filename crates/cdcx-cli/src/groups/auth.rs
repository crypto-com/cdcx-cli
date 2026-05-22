use cdcx_core::api_client::ApiClient;
use cdcx_core::auth::Credentials;
use cdcx_core::config::{check_config_permissions, Config};
use cdcx_core::env::Environment;
use cdcx_core::error::CdcxError;
use std::collections::HashMap;
use std::str::FromStr;

pub async fn run_auth_list() -> Result<(), CdcxError> {
    let path = Config::default_path()
        .ok_or_else(|| CdcxError::Config("Cannot determine home directory".into()))?;

    if !path.exists() {
        return Err(CdcxError::Config(format!(
            "No config file found at {}. Run 'cdcx setup' first.",
            path.display()
        )));
    }

    check_config_permissions(&path)?;

    let content = std::fs::read_to_string(&path)
        .map_err(|e| CdcxError::Config(format!("Failed to read config: {}", e)))?;
    let config = Config::parse(&content)?;

    let mut profiles: Vec<(String, cdcx_core::config::ProfileConfig)> = Vec::new();

    if let Some(ref default) = config.default {
        profiles.push(("default".to_string(), default.clone()));
    }

    if let Some(ref map) = config.profiles {
        for (name, profile) in map {
            if name == "default" && config.default.is_some() {
                continue;
            }
            profiles.push((name.clone(), profile.clone()));
        }
    }

    if profiles.is_empty() {
        return Err(CdcxError::Config(
            "No profiles found in config. Run 'cdcx setup' first.".into(),
        ));
    }

    println!("Profiles in {}:\n", path.display());

    // Cache auth check results by (api_key, environment)
    let mut cache: HashMap<(String, String, String), String> = HashMap::new();

    for (name, profile) in &profiles {
        // Apply this profile's env overrides, clearing any from a prior iteration.
        clear_cdcx_env_overrides();
        profile.apply_env();

        let env = Environment::from_str(&profile.environment).unwrap_or(Environment::Production);
        let masked_key = mask_key(&profile.api_key);

        let rest_url = std::env::var("CDCX_REST_URL").unwrap_or_default();
        let cache_key = (
            profile.api_key.clone(),
            profile.environment.clone(),
            rest_url,
        );
        let auth_status = if let Some(cached) = cache.get(&cache_key) {
            cached.clone()
        } else {
            let status = check_auth(&profile.api_key, &profile.api_secret, env).await;
            cache.insert(cache_key, status.clone());
            status
        };

        println!("  [{name}]");
        println!("    environment: {}", profile.environment);
        println!("    api_key:     {masked_key}");
        println!("    auth:        {auth_status}");
        println!();
    }
    clear_cdcx_env_overrides();

    Ok(())
}

async fn check_auth(api_key: &str, api_secret: &str, env: Environment) -> String {
    let creds = Credentials::from_parts(api_key.to_string(), api_secret.to_string());
    let client = ApiClient::new(Some(creds), env);
    match client
        .request("private/user-balance", serde_json::json!({}))
        .await
    {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("error ({})", e),
    }
}

fn clear_cdcx_env_overrides() {
    std::env::remove_var("CDCX_REST_URL");
    std::env::remove_var("CDCX_WS_MARKET_URL");
    std::env::remove_var("CDCX_WS_USER_URL");
}

fn mask_key(key: &str) -> String {
    if key.len() <= 6 {
        return "***".to_string();
    }
    let visible = &key[..4];
    format!("{}...{}", visible, &key[key.len() - 2..])
}
