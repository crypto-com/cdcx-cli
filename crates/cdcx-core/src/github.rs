const REPO: &str = env!("CARGO_PKG_REPOSITORY");

fn owner_repo() -> &'static str {
    REPO.strip_prefix("https://github.com/").unwrap()
}

pub fn html(path: &str) -> String {
    format!("{}/{}", REPO, path.trim_start_matches('/'))
}

pub fn api(path: &str) -> String {
    format!(
        "https://api.github.com/repos/{}/{}",
        owner_repo(),
        path.trim_start_matches('/')
    )
}

pub fn raw(branch: &str, path: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/{}/refs/heads/{}/{}",
        owner_repo(),
        branch,
        path.trim_start_matches('/')
    )
}

pub fn release_download(tag: &str, asset: &str) -> String {
    format!(
        "{}/releases/download/{}/{}",
        REPO,
        tag,
        asset.trim_start_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html() {
        assert_eq!(html("releases/latest"), format!("{}/releases/latest", REPO));
    }

    #[test]
    fn test_api() {
        let url = api("releases/latest");
        assert!(url.starts_with("https://api.github.com/repos/"));
        assert!(url.ends_with("/releases/latest"));
    }

    #[test]
    fn test_raw() {
        let url = raw("main", "schemas/configs/tui.json");
        assert!(url.contains("/refs/heads/main/"));
        assert!(url.ends_with("/schemas/configs/tui.json"));
    }

    #[test]
    fn test_release_download() {
        let url = release_download("v1.0.0", "cdcx-linux.tar.gz");
        assert!(url.contains("/releases/download/v1.0.0/"));
        assert!(url.ends_with("cdcx-linux.tar.gz"));
    }

    #[test]
    fn test_no_double_slash() {
        assert!(!raw("main", "/schemas/foo.json").contains("main//"));
        assert!(!api("/releases/latest").contains("repos//"));
        assert!(!html("/releases").contains("cli//"));
    }
}
