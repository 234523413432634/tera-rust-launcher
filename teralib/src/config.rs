use std::env;
use std::path::{Path, PathBuf};

use ini::Ini;
use once_cell::sync::Lazy;
use serde_json::Value;

/// Compiled-in fallbacks. Nothing here should name a concrete server: the
/// values that identify an installation (`server_url`, `client_ver`) are read
/// at runtime from the `[game]` section of the `config.ini` that ships next to
/// the launcher. These defaults only keep the launcher functional when a key is
/// missing from `config.ini`.
///
/// The URL entries are templates: `{server}` is replaced with the resolved
/// server URL, so pointing the launcher at another server is a one-line change
/// in `config.ini`.
const CONFIG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/config/config.json"
));

static CONFIG_JSON: Lazy<Value> =
    Lazy::new(|| serde_json::from_str(CONFIG).expect("Failed to parse config"));

/// Value accepted by `[game] path` meaning "the folder the launcher lives in".
pub const RELATIVE_PATH_MARKER: &str = "relative";

/// The `[game]` values read from `config.ini`, loaded once on first use.
struct IniConfig {
    server_url: Option<String>,
    client_ver: Option<String>,
}

static INI_CONFIG: Lazy<IniConfig> = Lazy::new(load_ini_config);

/// Locates `config.ini`.
///
/// The executable's own directory wins so that two launchers installed side by
/// side (e.g. `F:\Tera\Tera100` and `F:\Tera\Tera71`) each pick up their own
/// configuration no matter what the working directory happens to be.
pub fn find_config_file() -> Option<PathBuf> {
    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let config_in_exe_dir = exe_dir.join("config.ini");
            if config_in_exe_dir.exists() {
                return Some(config_in_exe_dir);
            }
        }
    }

    let current_dir = env::current_dir().ok()?;
    let config_in_current = current_dir.join("config.ini");
    if config_in_current.exists() {
        return Some(config_in_current);
    }

    let parent_dir = current_dir.parent()?;
    let config_in_parent = parent_dir.join("config.ini");
    if config_in_parent.exists() {
        return Some(config_in_parent);
    }

    None
}

/// Directory the launcher executable lives in.
pub fn launcher_dir() -> Option<PathBuf> {
    env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

fn load_ini_config() -> IniConfig {
    let empty = IniConfig {
        server_url: None,
        client_ver: None,
    };

    let path = match find_config_file() {
        Some(path) => path,
        None => return empty,
    };
    let conf = match Ini::load_from_file(&path) {
        Ok(conf) => conf,
        Err(_) => return empty,
    };
    let section = match conf.section(Some("game")) {
        Some(section) => section,
        None => return empty,
    };

    let read = |key: &str| -> Option<String> {
        section
            .get(key)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };

    IniConfig {
        server_url: read("server_url"),
        client_ver: read("client_ver"),
    }
}

/// Turns a bare host (`192.168.6.129`, `play.example.com:8080`) into a usable
/// base URL. An explicit scheme is kept as-is, and any trailing slash is
/// dropped so the templates can concatenate paths safely.
fn normalize_server_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{}", trimmed)
    }
}

/// The base server URL: `[game] server_url` from `config.ini`, falling back to
/// the compiled-in `SERVER_URL`.
pub fn server_url() -> String {
    if let Some(url) = INI_CONFIG.server_url.as_deref() {
        return normalize_server_url(url);
    }
    normalize_server_url(CONFIG_JSON["SERVER_URL"].as_str().unwrap_or_default())
}

/// The game client version: `[game] client_ver` from `config.ini`, falling back
/// to the compiled-in `CLIENT_VERSION`.
pub fn client_version() -> String {
    if let Some(version) = INI_CONFIG.client_ver.as_deref() {
        return version.to_string();
    }
    CONFIG_JSON["CLIENT_VERSION"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Looks up a configuration value, applying the `config.ini` overrides and
/// expanding the `{server}` placeholder.
///
/// Returns `None` when the key is not known at all.
pub fn try_get_config_value(key: &str) -> Option<String> {
    match key {
        "SERVER_URL" => return Some(server_url()),
        "CLIENT_VERSION" => return Some(client_version()),
        _ => {}
    }

    let template = CONFIG_JSON[key].as_str()?;
    Some(template.replace("{server}", &server_url()))
}

pub fn get_config_value(key: &str) -> String {
    try_get_config_value(key)
        .unwrap_or_else(|| panic!("{} must be set in config.json", key))
}

fn is_separator(c: char) -> bool {
    c == '/' || c == '\\'
}

/// Resolves the `[game] path` value against the directory holding `config.ini`.
///
/// `relative` (and `.` or an empty value) means "use the launcher's own
/// folder", so the same `config.ini` can be dropped into any install directory.
/// Any other relative path is joined onto that folder; absolute paths are used
/// unchanged.
pub fn resolve_game_path(raw: &str, config_dir: &Path) -> PathBuf {
    let trimmed = raw.trim().trim_matches('"');

    if trimmed.is_empty() || trimmed == "." {
        return config_dir.to_path_buf();
    }

    // `relative`, optionally followed by a sub-directory: `relative\Client`.
    // The separator check keeps a real folder that merely starts with the same
    // letters (`relativity\Tera`) from being swallowed by the marker.
    if trimmed.len() >= RELATIVE_PATH_MARKER.len()
        && trimmed[..RELATIVE_PATH_MARKER.len()].eq_ignore_ascii_case(RELATIVE_PATH_MARKER)
    {
        let tail = &trimmed[RELATIVE_PATH_MARKER.len()..];
        if tail.is_empty() {
            return config_dir.to_path_buf();
        }
        if tail.starts_with(is_separator) {
            let sub = tail.trim_start_matches(is_separator).replace('\\', "/");
            return if sub.is_empty() {
                config_dir.to_path_buf()
            } else {
                config_dir.join(sub)
            };
        }
    }

    if looks_absolute(trimmed) {
        PathBuf::from(trimmed)
    } else {
        config_dir.join(trimmed)
    }
}

/// Whether a configured path should be taken as-is rather than resolved
/// against the launcher folder.
///
/// `Path::is_absolute` is platform-specific, and the Linux build still deals in
/// Windows-style game paths, so drive letters and UNC shares are recognised
/// explicitly.
fn looks_absolute(value: &str) -> bool {
    if Path::new(value).is_absolute() {
        return true;
    }
    let bytes = value.as_bytes();
    // Drive-qualified, e.g. `F:\Tera` or `F:/Tera`
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    // UNC share
    value.starts_with("\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        PathBuf::from(if cfg!(windows) {
            r"F:\Tera\Tera100"
        } else {
            "/opt/tera100"
        })
    }

    #[test]
    fn relative_marker_resolves_to_launcher_folder() {
        assert_eq!(resolve_game_path("relative", &dir()), dir());
        assert_eq!(resolve_game_path("  Relative ", &dir()), dir());
        assert_eq!(resolve_game_path(".", &dir()), dir());
        assert_eq!(resolve_game_path("", &dir()), dir());
    }

    #[test]
    fn relative_marker_accepts_a_subfolder() {
        assert_eq!(
            resolve_game_path(r"relative\Client", &dir()),
            dir().join("Client")
        );
    }

    #[test]
    fn absolute_paths_are_untouched() {
        assert_eq!(
            resolve_game_path(r"F:\Tera\Tera71", &dir()),
            PathBuf::from(r"F:\Tera\Tera71")
        );
        assert_eq!(
            resolve_game_path(r"\\nas\tera", &dir()),
            PathBuf::from(r"\\nas\tera")
        );
    }

    #[test]
    fn a_folder_starting_with_relative_is_not_the_marker() {
        assert_eq!(
            resolve_game_path("relativity", &dir()),
            dir().join("relativity")
        );
    }

    #[test]
    fn server_urls_gain_a_scheme_and_lose_trailing_slashes() {
        assert_eq!(normalize_server_url("192.168.6.129"), "http://192.168.6.129");
        assert_eq!(normalize_server_url("192.168.6.129/"), "http://192.168.6.129");
        assert_eq!(
            normalize_server_url("https://play.example.com"),
            "https://play.example.com"
        );
        assert_eq!(normalize_server_url("  "), "");
    }

    #[test]
    fn url_templates_expand_the_server_placeholder() {
        let server = server_url();
        assert_eq!(
            try_get_config_value("FILE_SERVER_URL"),
            Some(format!("{}/public", server))
        );
        assert_eq!(try_get_config_value("NOT_A_KEY"), None);
    }
}
