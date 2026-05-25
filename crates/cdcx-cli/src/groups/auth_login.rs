use cdcx_core::config::{self, Config, ProfileConfig};
use cdcx_core::error::CdcxError;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Deserialize)]
struct PollResponse {
    code: i64,
    result: Option<PollResult>,
}

#[derive(Deserialize)]
struct PollResult {
    api_key: String,
    secret_key: String,
}

#[derive(Serialize, Deserialize)]
struct ConfigFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<ProfileConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profiles: Option<HashMap<String, ProfileConfig>>,
    #[serde(default, skip_serializing_if = "is_false")]
    disable_update_check: bool,
}

fn is_false(v: &bool) -> bool {
    !v
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("cdcx")
        .join("config.toml")
}

fn oauth_url(environment: &str, session_id: &str) -> String {
    match environment {
        "uat" => format!(
            "https://dpre-xweb.3ona.co/exchange/authorize_broker?broker_id=cdcx-cli\
             &redirect_uri=https%3A%2F%2Fx-growth.crypto.com%2Fstatic%2Fcli%2Fauthenticate%2Fuat%3Fid%3D{session_id}\
             &broker_reference_id={session_id}"
        ),
        _ => format!(
            "https://crypto.com/exchange/authorize_broker?broker_id=cdcx-cli\
             &redirect_uri=https%3A%2F%2Fx-growth.crypto.com%2Fstatic%2Fcli%2Fauthenticate%3Fid%3D{session_id}\
             &broker_reference_id={session_id}"
        ),
    }
}

fn poll_url(environment: &str) -> &'static str {
    match environment {
        "uat" => "https://x-growth.crypto.com/static/cli/authenticate/uat",
        _ => "https://x-growth.crypto.com/static/cli/authenticate",
    }
}

fn prompt(label: &str) -> String {
    print!("{}", label);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}

fn prompt_choice(label: &str, options: &[&str], default: usize) -> usize {
    println!("{}", label);
    for (i, opt) in options.iter().enumerate() {
        let marker = if i == default { " (default)" } else { "" };
        println!("  [{}] {}{}", i + 1, opt, marker);
    }
    let input = prompt(&format!("Choice [{}]: ", default + 1));
    if input.is_empty() {
        return default;
    }
    input
        .parse::<usize>()
        .unwrap_or(default + 1)
        .saturating_sub(1)
        .min(options.len() - 1)
}

pub async fn run_auth_login() -> Result<(), CdcxError> {
    println!();
    println!("  cdcx auth login");
    println!("  ───────────────");
    println!("  Authenticate via browser-based OAuth.");
    println!();

    // Step 1: Environment selection
    let env_choice = prompt_choice("Environment:", &["production", "uat (sandbox/testnet)"], 0);
    let environment = match env_choice {
        0 => "production",
        _ => "uat",
    };

    // Step 2: Generate crypto-safe session ID
    let session_id = Uuid::new_v4().to_string();

    // Step 3: Open browser
    let url = oauth_url(environment, &session_id);
    println!();
    println!("  Opening browser for authentication...");
    println!("  {}", url);

    if let Err(_) = open::that(&url) {
        println!();
        println!("  Could not open browser automatically.");
        println!("  Please open the URL above manually.");
    }

    println!();
    println!("  Waiting for authentication... (press ESC to cancel)");
    println!();

    // Step 4: Poll with ESC cancellation
    let credentials = poll_for_credentials(environment, &session_id).await?;

    let (api_key, api_secret) = match credentials {
        Some(creds) => creds,
        None => {
            println!("  Cancelled.");
            return Ok(());
        }
    };

    // Step 5: Store credentials
    let profile_name = store_credentials(&api_key, &api_secret, environment)?;

    // Step 6: Success
    let masked_key = mask_key(&api_key);
    println!("  ✓ Authenticated successfully!");
    println!();
    println!("    profile:     {}", profile_name);
    println!("    environment: {}", environment);
    println!("    api_key:     {}", masked_key);
    println!();
    if profile_name == "default" {
        println!("  Test with: cdcx account summary");
    } else {
        println!(
            "  Test with: cdcx --profile {} account summary",
            profile_name
        );
    }
    println!();

    Ok(())
}

async fn poll_for_credentials(
    environment: &str,
    session_id: &str,
) -> Result<Option<(String, String)>, CdcxError> {
    let _guard = RawModeGuard::enable()
        .map_err(|e| CdcxError::Config(format!("Failed to enable raw mode: {}", e)))?;

    // Spawn a blocking thread for ESC key detection, communicating via channel
    let (esc_tx, mut esc_rx) = tokio::sync::mpsc::channel::<()>(1);
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    tokio::task::spawn_blocking(move || {
        while !cancel_clone.load(std::sync::atomic::Ordering::Relaxed) {
            if let Ok(true) = event::poll(Duration::from_millis(100)) {
                if let Ok(Event::Key(key_event)) = event::read() {
                    if key_event.kind == KeyEventKind::Press && key_event.code == KeyCode::Esc {
                        let _ = esc_tx.blocking_send(());
                        return;
                    }
                }
            }
        }
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .no_proxy()
        .pool_max_idle_per_host(0)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let base_url = poll_url(environment);
    let start = Instant::now();

    let result = loop {
        if start.elapsed() > POLL_TIMEOUT {
            break Err(CdcxError::Config(
                "Authentication timed out after 5 minutes".into(),
            ));
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let url = format!("{}?ts={}", base_url, ts);
        let body = serde_json::json!({ "session": session_id, "ts": ts });
        tokio::select! {
            resp = client.post(&url).json(&body)
                .header("Cache-Control", "no-cache, no-store")
                .header("Pragma", "no-cache")
                .send() => {
                if let Ok(r) = resp {
                    if r.status() == reqwest::StatusCode::OK {
                        if let Ok(poll_resp) = r.json::<PollResponse>().await {
                            if poll_resp.code == 0 {
                                if let Some(result) = poll_resp.result {
                                    break Ok(Some((result.api_key, result.secret_key)));
                                }
                            }
                        }
                    }
                }
            }
            _ = esc_rx.recv() => {
                break Ok(None);
            }
        }

        // Wait before next poll, still listening for ESC
        tokio::select! {
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
            _ = esc_rx.recv() => {
                break Ok(None);
            }
        }
    };

    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    result
}

fn store_credentials(
    api_key: &str,
    api_secret: &str,
    environment: &str,
) -> Result<String, CdcxError> {
    let path = config_path();

    let existing = if path.exists() {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| CdcxError::Config(format!("Failed to read config: {}", e)))?;
        Some(Config::parse(&content)?)
    } else {
        None
    };

    let new_profile = ProfileConfig {
        api_key: api_key.to_string(),
        api_secret: api_secret.to_string(),
        environment: environment.to_string(),
        envs: HashMap::new(),
    };

    let (profile_name, config_file) = match existing {
        Some(ref config) => {
            // Check for dedup by api_key
            if let Some((name, existing_profile)) = find_profile_by_api_key(config, api_key) {
                let updated = ProfileConfig {
                    api_key: api_key.to_string(),
                    api_secret: api_secret.to_string(),
                    environment: environment.to_string(),
                    envs: existing_profile.envs.clone(),
                };
                let cf = update_profile_in_config(config, &name, updated);
                (name, cf)
            } else {
                // No dedup match — assign new name
                let count = profile_count(config);
                let name = format!("account-{}", count);
                let cf = add_profile_to_config(config, &name, new_profile);
                (name, cf)
            }
        }
        None => {
            // No config exists — create with default profile
            let cf = ConfigFile {
                default: Some(new_profile),
                profiles: None,
                disable_update_check: false,
            };
            ("default".to_string(), cf)
        }
    };

    write_config(&path, &config_file)?;
    Ok(profile_name)
}

fn find_profile_by_api_key(config: &Config, api_key: &str) -> Option<(String, ProfileConfig)> {
    if let Some(ref default) = config.default {
        if default.api_key == api_key {
            return Some(("default".to_string(), default.clone()));
        }
    }
    if let Some(ref profiles) = config.profiles {
        for (name, profile) in profiles {
            if profile.api_key == api_key {
                return Some((name.clone(), profile.clone()));
            }
        }
    }
    None
}

fn profile_count(config: &Config) -> usize {
    let mut count = 0;
    if config.default.is_some() {
        count += 1;
    }
    if let Some(ref profiles) = config.profiles {
        count += profiles.len();
    }
    count
}

fn update_profile_in_config(config: &Config, name: &str, profile: ProfileConfig) -> ConfigFile {
    if name == "default" {
        ConfigFile {
            default: Some(profile),
            profiles: config.profiles.clone(),
            disable_update_check: config.disable_update_check,
        }
    } else {
        let mut profiles = config.profiles.clone().unwrap_or_default();
        profiles.insert(name.to_string(), profile);
        ConfigFile {
            default: config.default.clone(),
            profiles: Some(profiles),
            disable_update_check: config.disable_update_check,
        }
    }
}

fn add_profile_to_config(config: &Config, name: &str, profile: ProfileConfig) -> ConfigFile {
    let mut profiles = config.profiles.clone().unwrap_or_default();
    profiles.insert(name.to_string(), profile);
    ConfigFile {
        default: config.default.clone(),
        profiles: Some(profiles),
        disable_update_check: config.disable_update_check,
    }
}

fn write_config(path: &PathBuf, config_file: &ConfigFile) -> Result<(), CdcxError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CdcxError::Config(format!("Failed to create config directory: {}", e)))?;
        config::set_dir_owner_only(parent)
            .map_err(|e| CdcxError::Config(format!("Failed to secure config directory: {}", e)))?;
    }

    let toml_content = toml::to_string_pretty(config_file)
        .map_err(|e| CdcxError::Config(format!("Failed to serialize config: {}", e)))?;

    let schema_url = cdcx_core::github::raw("main", "schemas/configs/config.json");
    let content = format!("#:schema {}\n\n{}", schema_url, toml_content);

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|e| CdcxError::Config(format!("Failed to open config file: {}", e)))?;
    file.write_all(content.as_bytes())
        .map_err(|e| CdcxError::Config(format!("Failed to write config: {}", e)))?;
    drop(file);
    config::set_file_owner_only(path)
        .map_err(|e| CdcxError::Config(format!("Failed to secure config file: {}", e)))?;

    Ok(())
}

fn mask_key(key: &str) -> String {
    if key.len() <= 6 {
        return "***".to_string();
    }
    format!("{}...{}", &key[..4], &key[key.len() - 2..])
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── URL generation ───

    #[test]
    fn test_oauth_url_production() {
        let url = oauth_url("production", "abc-123");
        assert!(url.contains("broker_id=cdcx-cli"));
        assert!(url.contains("broker_reference_id=abc-123"));
        assert!(url.contains("authenticate%3Fid%3Dabc-123"));
        assert!(!url.contains("/uat"));
    }

    #[test]
    fn test_oauth_url_uat() {
        let url = oauth_url("uat", "abc-123");
        assert!(url.contains("broker_id=cdcx-cli"));
        assert!(url.contains("broker_reference_id=abc-123"));
        assert!(url.contains("authenticate%2Fuat%3Fid%3Dabc-123"));
    }

    #[test]
    fn test_oauth_url_uses_session_id_in_both_params() {
        let url = oauth_url("production", "my-uuid-value");
        assert_eq!(
            url.matches("my-uuid-value").count(),
            2,
            "session ID should appear in both redirect_uri and broker_reference_id"
        );
    }

    #[test]
    fn test_poll_url_production() {
        assert_eq!(
            poll_url("production"),
            "https://x-growth.crypto.com/static/cli/authenticate"
        );
    }

    #[test]
    fn test_poll_url_uat() {
        assert_eq!(
            poll_url("uat"),
            "https://x-growth.crypto.com/static/cli/authenticate/uat"
        );
    }

    #[test]
    fn test_poll_url_unknown_defaults_to_production() {
        assert_eq!(poll_url("anything"), poll_url("production"));
    }

    // ─── Profile counting ───

    #[test]
    fn test_profile_count_empty() {
        let config = Config::parse("").unwrap();
        assert_eq!(profile_count(&config), 0);
    }

    #[test]
    fn test_profile_count_default_only() {
        let config = Config::parse(
            "[default]\napi_key = \"k\"\napi_secret = \"s\"\nenvironment = \"production\"\n",
        )
        .unwrap();
        assert_eq!(profile_count(&config), 1);
    }

    #[test]
    fn test_profile_count_named_only() {
        let config = Config::parse(
            "[profiles.one]\napi_key = \"k\"\napi_secret = \"s\"\nenvironment = \"uat\"\n\
             [profiles.two]\napi_key = \"k2\"\napi_secret = \"s2\"\nenvironment = \"production\"\n",
        )
        .unwrap();
        assert_eq!(profile_count(&config), 2);
    }

    #[test]
    fn test_profile_count_mixed() {
        let config = Config::parse(
            "[default]\napi_key = \"k\"\napi_secret = \"s\"\nenvironment = \"production\"\n\
             [profiles.other]\napi_key = \"k2\"\napi_secret = \"s2\"\nenvironment = \"uat\"\n",
        )
        .unwrap();
        assert_eq!(profile_count(&config), 2);
    }

    // ─── Profile lookup by api_key ───

    #[test]
    fn test_find_profile_by_api_key_default() {
        let config = Config::parse(
            "[default]\napi_key = \"target\"\napi_secret = \"s\"\nenvironment = \"production\"\n",
        )
        .unwrap();
        let result = find_profile_by_api_key(&config, "target");
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "default");
    }

    #[test]
    fn test_find_profile_by_api_key_named() {
        let config = Config::parse(
            "[default]\napi_key = \"k1\"\napi_secret = \"s\"\nenvironment = \"production\"\n\
             [profiles.myprofile]\napi_key = \"target\"\napi_secret = \"s2\"\nenvironment = \"uat\"\n",
        )
        .unwrap();
        let result = find_profile_by_api_key(&config, "target");
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "myprofile");
    }

    #[test]
    fn test_find_profile_by_api_key_not_found() {
        let config = Config::parse(
            "[default]\napi_key = \"k1\"\napi_secret = \"s\"\nenvironment = \"production\"\n",
        )
        .unwrap();
        assert!(find_profile_by_api_key(&config, "nonexistent").is_none());
    }

    #[test]
    fn test_find_profile_by_api_key_prefers_default() {
        let config = Config::parse(
            "[default]\napi_key = \"same_key\"\napi_secret = \"s1\"\nenvironment = \"production\"\n\
             [profiles.named]\napi_key = \"same_key\"\napi_secret = \"s2\"\nenvironment = \"uat\"\n",
        )
        .unwrap();
        let (name, _) = find_profile_by_api_key(&config, "same_key").unwrap();
        assert_eq!(name, "default");
    }

    // ─── Config update (dedup) ───

    #[test]
    fn test_dedup_preserves_envs() {
        let mut envs = HashMap::new();
        envs.insert("CDCX_REST_URL".to_string(), "http://custom".to_string());

        let config = Config {
            default: Some(ProfileConfig {
                api_key: "target_key".into(),
                api_secret: "old_secret".into(),
                environment: "uat".into(),
                envs: envs.clone(),
            }),
            profiles: None,
            disable_update_check: false,
        };

        let (name, existing) = find_profile_by_api_key(&config, "target_key").unwrap();
        assert_eq!(name, "default");

        let updated = ProfileConfig {
            api_key: "target_key".into(),
            api_secret: "new_secret".into(),
            environment: "production".into(),
            envs: existing.envs.clone(),
        };

        let cf = update_profile_in_config(&config, &name, updated);
        let default = cf.default.unwrap();
        assert_eq!(default.api_secret, "new_secret");
        assert_eq!(default.environment, "production");
        assert_eq!(default.envs.get("CDCX_REST_URL").unwrap(), "http://custom");
    }

    #[test]
    fn test_dedup_updates_named_profile() {
        let config = Config::parse(
            "[default]\napi_key = \"k1\"\napi_secret = \"s1\"\nenvironment = \"production\"\n\
             [profiles.account-1]\napi_key = \"target\"\napi_secret = \"old\"\nenvironment = \"uat\"\n",
        )
        .unwrap();

        let (name, _) = find_profile_by_api_key(&config, "target").unwrap();
        assert_eq!(name, "account-1");

        let updated = ProfileConfig {
            api_key: "target".into(),
            api_secret: "new_secret".into(),
            environment: "production".into(),
            envs: HashMap::new(),
        };
        let cf = update_profile_in_config(&config, &name, updated);

        let profiles = cf.profiles.unwrap();
        let p = profiles.get("account-1").unwrap();
        assert_eq!(p.api_secret, "new_secret");
        assert_eq!(p.environment, "production");
        // default untouched
        assert_eq!(cf.default.unwrap().api_key, "k1");
    }

    #[test]
    fn test_update_preserves_disable_update_check() {
        let config = Config {
            default: Some(ProfileConfig {
                api_key: "k".into(),
                api_secret: "s".into(),
                environment: "production".into(),
                envs: HashMap::new(),
            }),
            profiles: None,
            disable_update_check: true,
        };

        let updated = ProfileConfig {
            api_key: "k".into(),
            api_secret: "new_s".into(),
            environment: "production".into(),
            envs: HashMap::new(),
        };
        let cf = update_profile_in_config(&config, "default", updated);
        assert!(cf.disable_update_check);
    }

    // ─── Config add (new profile) ───

    #[test]
    fn test_add_profile_to_empty_profiles() {
        let config = Config {
            default: Some(ProfileConfig {
                api_key: "k1".into(),
                api_secret: "s1".into(),
                environment: "production".into(),
                envs: HashMap::new(),
            }),
            profiles: None,
            disable_update_check: false,
        };

        let new_profile = ProfileConfig {
            api_key: "k2".into(),
            api_secret: "s2".into(),
            environment: "uat".into(),
            envs: HashMap::new(),
        };
        let cf = add_profile_to_config(&config, "account-1", new_profile);

        assert!(cf.default.is_some());
        let profiles = cf.profiles.unwrap();
        assert!(profiles.contains_key("account-1"));
        assert_eq!(profiles["account-1"].api_key, "k2");
    }

    #[test]
    fn test_add_profile_preserves_existing() {
        let config = Config::parse(
            "[default]\napi_key = \"k1\"\napi_secret = \"s1\"\nenvironment = \"production\"\n\
             [profiles.existing]\napi_key = \"k2\"\napi_secret = \"s2\"\nenvironment = \"uat\"\n",
        )
        .unwrap();

        let new_profile = ProfileConfig {
            api_key: "k3".into(),
            api_secret: "s3".into(),
            environment: "production".into(),
            envs: HashMap::new(),
        };
        let cf = add_profile_to_config(&config, "account-2", new_profile);

        let profiles = cf.profiles.unwrap();
        assert!(profiles.contains_key("existing"));
        assert!(profiles.contains_key("account-2"));
        assert_eq!(profiles.len(), 2);
    }

    // ─── Profile naming logic ───

    #[test]
    fn test_naming_no_config_uses_default() {
        let new_profile = ProfileConfig {
            api_key: "k".into(),
            api_secret: "s".into(),
            environment: "production".into(),
            envs: HashMap::new(),
        };
        let cf = ConfigFile {
            default: Some(new_profile),
            profiles: None,
            disable_update_check: false,
        };
        assert!(cf.default.is_some());
        assert!(cf.profiles.is_none());
    }

    #[test]
    fn test_naming_with_existing_default_uses_account_1() {
        let config = Config::parse(
            "[default]\napi_key = \"existing\"\napi_secret = \"s\"\nenvironment = \"production\"\n",
        )
        .unwrap();
        let count = profile_count(&config);
        let name = format!("account-{}", count);
        assert_eq!(name, "account-1");
    }

    #[test]
    fn test_naming_with_two_profiles_uses_account_2() {
        let config = Config::parse(
            "[default]\napi_key = \"k1\"\napi_secret = \"s\"\nenvironment = \"production\"\n\
             [profiles.account-1]\napi_key = \"k2\"\napi_secret = \"s2\"\nenvironment = \"uat\"\n",
        )
        .unwrap();
        let count = profile_count(&config);
        let name = format!("account-{}", count);
        assert_eq!(name, "account-2");
    }

    // ─── Serialization roundtrip ───

    #[test]
    fn test_config_file_serialization_roundtrip() {
        let cf = ConfigFile {
            default: Some(ProfileConfig {
                api_key: "my_key".into(),
                api_secret: "my_secret".into(),
                environment: "production".into(),
                envs: HashMap::new(),
            }),
            profiles: None,
            disable_update_check: false,
        };
        let toml_str = toml::to_string_pretty(&cf).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        let p = parsed.profile(None).unwrap();
        assert_eq!(p.api_key, "my_key");
        assert_eq!(p.api_secret, "my_secret");
        assert_eq!(p.environment, "production");
    }

    #[test]
    fn test_config_file_with_profiles_roundtrip() {
        let mut profiles = HashMap::new();
        profiles.insert(
            "account-1".to_string(),
            ProfileConfig {
                api_key: "k2".into(),
                api_secret: "s2".into(),
                environment: "uat".into(),
                envs: HashMap::new(),
            },
        );
        let cf = ConfigFile {
            default: Some(ProfileConfig {
                api_key: "k1".into(),
                api_secret: "s1".into(),
                environment: "production".into(),
                envs: HashMap::new(),
            }),
            profiles: Some(profiles),
            disable_update_check: true,
        };
        let toml_str = toml::to_string_pretty(&cf).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.profile(None).unwrap().api_key, "k1");
        assert_eq!(parsed.profile(Some("account-1")).unwrap().api_key, "k2");
        assert!(parsed.disable_update_check);
    }

    #[test]
    fn test_empty_envs_not_serialized() {
        let cf = ConfigFile {
            default: Some(ProfileConfig {
                api_key: "k".into(),
                api_secret: "s".into(),
                environment: "production".into(),
                envs: HashMap::new(),
            }),
            profiles: None,
            disable_update_check: false,
        };
        let toml_str = toml::to_string_pretty(&cf).unwrap();
        assert!(
            !toml_str.contains("envs"),
            "empty envs should not appear in serialized TOML"
        );
    }

    #[test]
    fn test_nonempty_envs_preserved_in_roundtrip() {
        let mut envs = HashMap::new();
        envs.insert("CDCX_REST_URL".to_string(), "http://localhost".to_string());
        let cf = ConfigFile {
            default: Some(ProfileConfig {
                api_key: "k".into(),
                api_secret: "s".into(),
                environment: "production".into(),
                envs,
            }),
            profiles: None,
            disable_update_check: false,
        };
        let toml_str = toml::to_string_pretty(&cf).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        let p = parsed.profile(None).unwrap();
        assert_eq!(p.envs.get("CDCX_REST_URL").unwrap(), "http://localhost");
    }

    // ─── Poll response deserialization ───

    #[test]
    fn test_poll_response_success() {
        let json = r#"{"id":1,"method":"private/broker/create-fast-api-key","code":0,"result":{"ts":1779687323265,"api_key":"the_key","secret_key":"the_secret","enabled_trading":true,"enabled_withdrawal":false}}"#;
        let resp: PollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let result = resp.result.unwrap();
        assert_eq!(result.api_key, "the_key");
        assert_eq!(result.secret_key, "the_secret");
    }

    #[test]
    fn test_poll_response_non_zero_code() {
        let json = r#"{"id":1,"method":"something","code":10001}"#;
        let resp: PollResponse = serde_json::from_str(json).unwrap();
        assert_ne!(resp.code, 0);
        assert!(resp.result.is_none());
    }

    #[test]
    fn test_poll_response_with_extra_fields() {
        let json = r#"{"id":1,"method":"private/broker/create-fast-api-key","code":0,"result":{"ts":123,"api_key":"k","secret_key":"s","enabled_trading":true,"enabled_withdrawal":false,"extra":"ignored"}}"#;
        let resp: PollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert_eq!(resp.result.unwrap().api_key, "k");
    }

    // ─── Mask key ───

    #[test]
    fn test_mask_key() {
        assert_eq!(mask_key("abcdefghij"), "abcd...ij");
        assert_eq!(mask_key("short"), "***");
        assert_eq!(mask_key("1234567"), "1234...67");
    }

    #[test]
    fn test_mask_key_exactly_six() {
        assert_eq!(mask_key("123456"), "***");
    }

    #[test]
    fn test_mask_key_seven() {
        assert_eq!(mask_key("1234567"), "1234...67");
    }

    #[test]
    fn test_mask_key_empty() {
        assert_eq!(mask_key(""), "***");
    }
}
