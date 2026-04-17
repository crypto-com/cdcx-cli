use crate::error::ErrorEnvelope;
use crate::output::OutputFormat;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyTier {
    Read,
    SensitiveRead,
    Mutate,
    Dangerous,
}

impl SafetyTier {
    pub fn from_method(method: &str) -> Self {
        // Public endpoints are always Read
        if method.starts_with("public/") {
            return Self::Read;
        }

        // Dangerous operations
        match method {
            "private/cancel-all-orders"
            | "private/advanced/cancel-all-orders"
            | "private/create-withdrawal"
            | "private/fiat/fiat-create-withdraw" => Self::Dangerous,

            // Mutate operations
            "private/create-order"
            | "private/create-order-list"
            | "private/cancel-order"
            | "private/cancel-order-list"
            | "private/amend-order"
            | "private/close-position"
            | "private/advanced/create-order"
            | "private/advanced/create-oco"
            | "private/advanced/cancel-oco"
            | "private/advanced/create-oto"
            | "private/advanced/cancel-oto"
            | "private/advanced/create-otoco"
            | "private/advanced/cancel-otoco"
            | "private/advanced/cancel-order"
            | "private/advanced/amend-order"
            | "private/advanced/create-order-list"
            | "private/advanced/cancel-order-list"
            | "private/change-account-leverage"
            | "private/change-account-settings"
            | "private/create-isolated-margin-transfer"
            | "private/change-isolated-margin-leverage"
            | "private/staking/stake"
            | "private/staking/unstake"
            | "private/staking/convert"
            | "private/create-subaccount-transfer" => Self::Mutate,

            // Everything else that's private is SensitiveRead
            _ => Self::SensitiveRead,
        }
    }
}

/// Determine whether to prompt the user for confirmation.
/// Only prompts when: output is Table (human mode), tier is Mutate or Dangerous,
/// TTY is available, and neither --yes nor --dry-run is set.
pub fn should_prompt(
    tier: SafetyTier,
    is_tty: bool,
    yes_flag: bool,
    dry_run: bool,
    format: OutputFormat,
) -> bool {
    if format != OutputFormat::Table {
        return false;
    }
    if !is_tty {
        return false;
    }
    if yes_flag || dry_run {
        return false;
    }
    matches!(tier, SafetyTier::Mutate | SafetyTier::Dangerous)
}

/// Check acknowledged parameter for MCP gating.
/// Mutate: requires acknowledged=true
/// Dangerous: requires acknowledged=true AND allow_dangerous=true on server
pub fn check_acknowledged(
    tier: SafetyTier,
    acknowledged: bool,
    allow_dangerous: bool,
) -> Result<(), ErrorEnvelope> {
    match tier {
        SafetyTier::Read | SafetyTier::SensitiveRead => Ok(()),
        SafetyTier::Mutate => {
            if acknowledged {
                Ok(())
            } else {
                Err(ErrorEnvelope::safety(
                    "This operation modifies state. Set acknowledged=true to proceed.",
                ))
            }
        }
        SafetyTier::Dangerous => {
            if !allow_dangerous {
                Err(ErrorEnvelope::safety(
                    "This operation is dangerous. Start the MCP server with --allow-dangerous.",
                ))
            } else if !acknowledged {
                Err(ErrorEnvelope::safety(
                    "This dangerous operation requires acknowledged=true.",
                ))
            } else {
                Ok(())
            }
        }
    }
}

/// Generate dry-run output showing what would be sent without executing.
pub fn dry_run_output(method: &str, params: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "dry_run": true,
        "method": method,
        "params": params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safety_tier_from_method() {
        assert_eq!(
            SafetyTier::from_method("public/get-tickers"),
            SafetyTier::Read
        );
        assert_eq!(
            SafetyTier::from_method("private/get-accounts"),
            SafetyTier::SensitiveRead
        );
        assert_eq!(
            SafetyTier::from_method("private/create-order"),
            SafetyTier::Mutate
        );
        assert_eq!(
            SafetyTier::from_method("private/cancel-all-orders"),
            SafetyTier::Dangerous
        );
        assert_eq!(
            SafetyTier::from_method("private/create-withdrawal"),
            SafetyTier::Dangerous
        );
    }

    #[test]
    fn test_should_prompt() {
        // Never prompt for non-table output (JSON is default, agents shouldn't get prompts)
        assert!(!should_prompt(
            SafetyTier::Mutate,
            false,
            false,
            false,
            OutputFormat::Json
        ));
        // Prompt for table output on mutate
        assert!(should_prompt(
            SafetyTier::Mutate,
            true,
            false,
            false,
            OutputFormat::Table
        ));
        // --yes skips prompt
        assert!(!should_prompt(
            SafetyTier::Mutate,
            true,
            true,
            false,
            OutputFormat::Table
        ));
        // --dry-run skips prompt
        assert!(!should_prompt(
            SafetyTier::Mutate,
            true,
            false,
            true,
            OutputFormat::Table
        ));
        // Never prompt for Read
        assert!(!should_prompt(
            SafetyTier::Read,
            true,
            false,
            false,
            OutputFormat::Table
        ));
    }

    #[test]
    fn test_check_acknowledged() {
        // Mutate without acknowledged -> error
        assert!(check_acknowledged(SafetyTier::Mutate, false, false).is_err());
        // Mutate with acknowledged -> ok
        assert!(check_acknowledged(SafetyTier::Mutate, true, false).is_ok());
        // Dangerous without allow_dangerous -> error even with acknowledged
        assert!(check_acknowledged(SafetyTier::Dangerous, true, false).is_err());
        // Dangerous with both -> ok
        assert!(check_acknowledged(SafetyTier::Dangerous, true, true).is_ok());
        // Read -> always ok
        assert!(check_acknowledged(SafetyTier::Read, false, false).is_ok());
    }

    #[test]
    fn test_dry_run_output() {
        let output = dry_run_output("private/create-order", &serde_json::json!({"side": "BUY"}));
        assert_eq!(output["method"], "private/create-order");
        assert_eq!(output["params"]["side"], "BUY");
        assert_eq!(output["dry_run"], true);
    }

    #[test]
    fn test_catch_all_does_not_downgrade_private_methods() {
        // Verify that the catch-all doesn't silently downgrade private methods
        // Any unrecognized private method should be SensitiveRead, not Read
        let unknown_private = SafetyTier::from_method("private/unknown-operation");
        assert_ne!(
            unknown_private,
            SafetyTier::Read,
            "Private methods should never be Read tier"
        );
    }
}
