use std::{
    cmp::Ordering,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use arboard::Clipboard;
use quick_xml::{Reader as XmlReader, events::Event};
use rfd::{FileDialog, MessageDialog, MessageDialogResult, MessageLevel};
use slint::{Image, Model, ModelRc, SharedString, VecModel};
use zip::ZipArchive;

use super::{
    DirectoryConfig, DirectoryEntry, UrlRulesConfig, UserConfig, WebDavClient,
    available_destination, default_directory, join_remote_path, list_directory, load_user_config,
    normalize_remote_relative_path, remote_listing, resolve_path, save_user_config,
    transfer_remote_entry,
};

slint::include_modules!();

#[derive(Default)]
struct AppState {
    root: Option<PathBuf>,
    current: String,
    entries: Vec<DirectoryEntry>,
    selected: Option<String>,
    remote: Option<WebDavClient>,
    remote_id: String,
    config: UserConfig,
    config_load_failed: bool,
    editing_location: Option<String>,
    transfer: Option<(String, bool, String)>,
    opened_temp_files: Vec<tempfile::TempPath>,
}

enum PreviewResult {
    Text(String),
    Image(Image),
    External,
}

const MAX_OFFICE_XML_BYTES: u64 = 32 * 1024 * 1024;

fn normalize_webdav_endpoint(endpoint: &str) -> Result<String, String> {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if endpoint.is_empty() {
        return Err("Enter a server address".into());
    }
    Ok(endpoint.to_string())
}

fn webdav_location_name(endpoint: &str) -> String {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|endpoint| endpoint.host_str().map(str::to_owned))
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "WebDAV".into())
}

fn is_url_label(label: &str) -> bool {
    label
        .get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"))
        || label
            .get(..8)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
}

fn location_display_name(location: &DirectoryConfig) -> String {
    let label = location.label.trim();
    if location.kind == "webdav" {
        if !label.is_empty() && !is_url_label(label) {
            return label.to_owned();
        }
        return webdav_location_name(&location.endpoint);
    }
    if !label.is_empty() {
        return label.to_owned();
    }
    Path::new(&location.path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| location.path.clone())
}

fn supported_system_locale(locale: &str) -> &'static str {
    let locale = locale.to_ascii_lowercase().replace('_', "-");
    if locale.starts_with("zh") {
        if ["tw", "hk", "mo", "hant"]
            .iter()
            .any(|region| locale.contains(region))
        {
            "zh-TW"
        } else {
            "zh-CN"
        }
    } else if locale.starts_with("fr") {
        "fr"
    } else if locale.starts_with("ja") {
        "ja"
    } else if locale.starts_with("ko") {
        "ko"
    } else {
        "en"
    }
}

fn system_locale() -> &'static str {
    supported_system_locale(&sys_locale::get_locale().unwrap_or_else(|| "en-US".to_owned()))
}

fn system_uses_dark_theme() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if let Ok(output) = Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "AppsUseLightTheme",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .stderr(std::process::Stdio::null())
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
            if text.contains("appsuselighttheme") && text.contains("0x0") {
                return true;
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .stderr(std::process::Stdio::null())
            .output()
        {
            return String::from_utf8_lossy(&output.stdout).trim() == "Dark";
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .stderr(std::process::Stdio::null())
            .output()
        {
            if String::from_utf8_lossy(&output.stdout).contains("dark") {
                return true;
            }
        }
    }
    false
}

impl AppState {
    fn open_directory(&mut self, path: &str) -> Result<(), String> {
        let candidate = PathBuf::from(path.trim());
        let root = fs::canonicalize(candidate).map_err(|error| error.to_string())?;
        if !root.is_dir() {
            return Err("The selected path is not a directory".into());
        }
        let entries = list_directory(&root, &root)?;
        let path = root.to_string_lossy().into_owned();
        let mut config = self.config.clone();
        config.active_directory = Some(path.clone());
        if !config
            .directories
            .iter()
            .any(|directory| directory.path == path)
        {
            config.directories.push(DirectoryConfig {
                path: path.clone(),
                kind: "local".into(),
                label: root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                endpoint: String::new(),
                username: String::new(),
                password: String::new(),
                rules: UrlRulesConfig::default(),
            });
        }
        if !self.config_load_failed {
            save_user_config(config.clone())?;
        }
        self.config = config;
        self.root = Some(root);
        self.remote = None;
        self.remote_id.clear();
        self.current.clear();
        self.entries = entries;
        self.selected = None;
        self.select_first_file();
        Ok(())
    }

    fn connect_remote(
        &mut self,
        endpoint: &str,
        username: &str,
        password: &str,
    ) -> Result<(), String> {
        let endpoint = normalize_webdav_endpoint(endpoint)?;
        let client = WebDavClient::new(&endpoint, username, password)?;
        let id = format!("webdav://{endpoint}");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let listing = runtime.block_on(remote_listing(&client, ""))?;
        let mut config = self.config.clone();
        config.active_directory = Some(id.clone());
        if let Some(directory) = config
            .directories
            .iter_mut()
            .find(|directory| directory.path == id)
        {
            directory.endpoint = endpoint.to_string();
            directory.username = username.to_string();
            directory.password = password.to_string();
            directory.kind = "webdav".into();
            if directory.label.trim().is_empty() || is_url_label(&directory.label) {
                directory.label = webdav_location_name(&endpoint);
            }
        } else {
            config.directories.push(DirectoryConfig {
                path: id.clone(),
                kind: "webdav".into(),
                label: webdav_location_name(&endpoint),
                endpoint: endpoint.to_string(),
                username: username.to_string(),
                password: password.to_string(),
                rules: UrlRulesConfig::default(),
            });
        }
        if !self.config_load_failed {
            save_user_config(config.clone())?;
        }
        self.config = config;
        self.root = None;
        self.remote = Some(client);
        self.remote_id = id.clone();
        self.current.clear();
        self.entries = listing.entries;
        self.selected = None;
        self.select_first_file();
        Ok(())
    }

    fn activate_location(&mut self, path: &str) -> Result<(), String> {
        let location = self
            .config
            .directories
            .iter()
            .find(|directory| directory.path == path)
            .cloned()
            .ok_or_else(|| "The selected location is no longer available".to_string())?;
        if location.kind == "webdav" {
            self.connect_remote(&location.endpoint, &location.username, &location.password)
        } else {
            self.open_directory(&location.path)
        }
    }

    fn remove_location(&mut self, path: &str) -> Result<(), String> {
        if !self
            .config
            .directories
            .iter()
            .any(|directory| directory.path == path)
        {
            return Err("The selected location is no longer available".into());
        }
        if self.config_load_failed {
            return Err("Settings could not be loaded; the location was not removed".into());
        }
        let was_active = self.config.active_directory.as_deref() == Some(path);
        let mut config = self.config.clone();
        let next_location = was_active
            .then(|| {
                config
                    .directories
                    .iter()
                    .find(|directory| directory.path != path)
                    .map(|directory| directory.path.clone())
            })
            .flatten();
        config
            .directories
            .retain(|directory| directory.path != path);
        if was_active {
            config.active_directory = None;
        }
        save_user_config(config.clone())?;
        self.config = config;
        if was_active {
            self.root = None;
            self.remote = None;
            self.remote_id.clear();
            self.entries.clear();
            self.current.clear();
            self.selected = None;
        }
        if self
            .transfer
            .as_ref()
            .is_some_and(|(_, _, source_location)| source_location == path)
        {
            self.transfer = None;
        }
        if self.editing_location.as_deref() == Some(path) {
            self.editing_location = None;
        }
        if let Some(next_location) = next_location {
            return self.activate_location(&next_location);
        }
        Ok(())
    }

    fn save_preferences(&mut self, locale: Option<String>, theme: &str) -> Result<(), String> {
        if self.config_load_failed {
            return Err("Settings could not be loaded; preferences were not saved".into());
        }
        if !matches!(theme, "system" | "light" | "dark") {
            return Err("Choose a supported appearance".into());
        }
        let mut config = self.config.clone();
        if let Some(locale) = locale {
            config.locale = Some(locale);
        }
        config.theme = Some(theme.into());
        save_user_config(config.clone())?;
        self.config = config;
        Ok(())
    }

    fn check_updates(&self) -> Result<String, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        runtime.block_on(async {
            let response = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(12))
                .build()
                .map_err(|error| error.to_string())?
                .get("https://api.github.com/repos/Kaiyuan/r2-list/releases/latest")
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "R2-List-App")
                .send()
                .await
                .map_err(|error| error.to_string())?
                .error_for_status()
                .map_err(|error| error.to_string())?;
            let payload: serde_json::Value =
                serde_json::from_slice(&response.bytes().await.map_err(|error| error.to_string())?)
                    .map_err(|error| error.to_string())?;
            let latest = payload
                .get("tag_name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "GitHub did not return a release tag".to_string())?
                .trim_start_matches(['v', 'V']);
            match compare_versions(env!("CARGO_PKG_VERSION"), latest)? {
                Ordering::Less => Ok(format!("Update available: {latest}")),
                Ordering::Equal | Ordering::Greater => Ok(format!("Up to date (latest: {latest})")),
            }
        })
    }

    fn check_github_connection(&self) -> Result<(), String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        runtime.block_on(async {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(12))
                .build()
                .map_err(|error| error.to_string())?
                .get("https://api.github.com/repos/Kaiyuan/r2-list")
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "R2-List-App")
                .send()
                .await
                .map_err(|error| error.to_string())?
                .error_for_status()
                .map_err(|error| error.to_string())?;
            Ok(())
        })
    }

    fn change_config_directory(&mut self, path: String) -> Result<super::ConfigLocation, String> {
        if self.config_load_failed {
            return Err(
                "Settings could not be loaded; the configuration folder was not changed".into(),
            );
        }
        let location = super::set_config_directory(path)?;
        save_user_config(self.config.clone())?;
        Ok(location)
    }

    fn refresh(&mut self) -> Result<(), String> {
        if let Some(client) = &self.remote {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            let listing = runtime.block_on(remote_listing(client, &self.current))?;
            self.entries = listing.entries;
            self.select_first_file();
            return Ok(());
        }
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| "Choose a folder first".to_string())?;
        let current = resolve_path(root, &self.current)?;
        self.entries = list_directory(root, &current)?;
        self.select_first_file();
        Ok(())
    }

    fn select_first_file(&mut self) {
        self.selected = self
            .entries
            .iter()
            .find(|entry| {
                entry.kind != "directory"
                    && (self.config.show_hidden || !is_ignored_entry(&entry.name))
            })
            .map(|entry| entry.path.clone());
    }

    fn navigate(&mut self, path: &str) -> Result<(), String> {
        let previous_path = std::mem::replace(&mut self.current, path.to_string());
        let previous_selection = self.selected.clone();
        if let Err(error) = self.refresh() {
            self.current = previous_path;
            self.selected = previous_selection;
            return Err(error);
        }
        Ok(())
    }

    fn current_path(&self) -> Result<PathBuf, String> {
        if self.remote.is_some() {
            return Err("This operation is not supported for a WebDAV location".into());
        }
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| "Choose a folder first".to_string())?;
        resolve_path(root, &self.current)
    }

    fn rules(&self) -> UrlRulesConfig {
        let path = self.active_location();
        self.config
            .directories
            .iter()
            .find(|directory| directory.path == path)
            .map(|directory| directory.rules.clone())
            .unwrap_or_default()
    }

    fn set_rules(
        &mut self,
        path: &str,
        label: String,
        base_url: String,
        prefix: String,
        bucket: String,
        template: String,
        endpoint: String,
        username: String,
        password: String,
    ) -> Result<String, String> {
        if self.config_load_failed {
            return Err("Settings could not be loaded; URL rules were not saved".into());
        }
        let rules = UrlRulesConfig {
            base_url,
            prefix,
            bucket: (!bucket.trim().is_empty()).then_some(bucket),
            template: (!template.trim().is_empty()).then_some(template),
        };
        build_object_url("r2-list-validation.txt", &rules)?;
        let is_remote = self
            .config
            .directories
            .iter()
            .find(|directory| directory.path == path)
            .ok_or_else(|| "The selected location is not in the settings".to_string())?
            .kind
            == "webdav";
        let endpoint = if is_remote {
            normalize_webdav_endpoint(&endpoint)?
        } else {
            String::new()
        };
        if is_remote {
            WebDavClient::new(&endpoint, &username, &password)?;
        }
        let new_path = if is_remote {
            format!("webdav://{endpoint}")
        } else {
            path.to_string()
        };
        if new_path != path
            && self
                .config
                .directories
                .iter()
                .any(|directory| directory.path == new_path)
        {
            return Err("A location for this WebDAV address already exists".into());
        }
        let was_active = self.config.active_directory.as_deref() == Some(path);
        let mut config = self.config.clone();
        let directory = config
            .directories
            .iter_mut()
            .find(|directory| directory.path == path)
            .ok_or_else(|| "The selected location is not in the settings".to_string())?;
        directory.path = new_path.clone();
        directory.rules = rules;
        if !label.trim().is_empty() {
            directory.label = label.trim().to_string();
        }
        if is_remote {
            directory.endpoint = endpoint;
            directory.username = username.trim().to_string();
            directory.password = password;
        }
        if was_active {
            config.active_directory = Some(new_path.clone());
        }
        save_user_config(config.clone())?;
        self.config = config;
        if was_active && is_remote {
            self.remote_id = new_path.clone();
        }
        if self.editing_location.as_deref() == Some(path) {
            self.editing_location = Some(new_path.clone());
        }
        Ok(new_path)
    }

    fn start_transfer(&mut self, mode: &str) -> Result<(), String> {
        let entry = self
            .selected_entry()
            .ok_or_else(|| "Select an item first".to_string())?;
        self.transfer = Some((entry.path.clone(), mode == "move", self.active_location()));
        Ok(())
    }

    fn cancel_transfer(&mut self) {
        self.transfer = None;
    }

    fn paste_transfer(&mut self) -> Result<(), String> {
        let destination = self.current.clone();
        self.paste_transfer_to(&destination)
    }

    fn paste_transfer_to(&mut self, destination: &str) -> Result<(), String> {
        let (source, moving, source_location) = self
            .transfer
            .clone()
            .ok_or_else(|| "Copy or move an item before pasting".to_string())?;
        if source_location != self.active_location() {
            return Err("Transfers must stay within the same location".into());
        }
        if let Some(client) = &self.remote {
            let destination = normalize_remote_relative_path(destination)?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            runtime.block_on(transfer_remote_entry(
                client,
                &source,
                &destination,
                if moving { b"MOVE" } else { b"COPY" },
                (!moving).then_some("infinity"),
            ))?;
        } else {
            let root = self
                .root
                .as_ref()
                .ok_or_else(|| "Choose a location first".to_string())?;
            let source_path = resolve_path(root, &source)?;
            let destination = resolve_path(root, destination)?;
            if !destination.is_dir() {
                return Err("The destination is not a directory".into());
            }
            if source_path.is_dir() && destination.starts_with(&source_path) {
                return Err("Cannot transfer a folder into itself".into());
            }
            let name = source_path
                .file_name()
                .ok_or_else(|| "The selected item has no name".to_string())?
                .to_string_lossy();
            let target = available_destination(&destination, &name);
            if moving {
                if fs::rename(&source_path, &target).is_err() {
                    super::copy_recursively(&source_path, &target)?;
                    if source_path.is_dir() {
                        fs::remove_dir_all(&source_path).map_err(|error| error.to_string())?;
                    } else {
                        fs::remove_file(&source_path).map_err(|error| error.to_string())?;
                    }
                }
            } else {
                super::copy_recursively(&source_path, &target)?;
            }
        }
        self.transfer = None;
        self.refresh()
    }

    fn selected_entry(&self) -> Option<&DirectoryEntry> {
        self.entries
            .iter()
            .find(|entry| Some(entry.path.as_str()) == self.selected.as_deref())
    }

    fn files_under(&self, entry: &DirectoryEntry) -> Result<Vec<DirectoryEntry>, String> {
        if entry.kind != "directory" {
            return Ok(vec![entry.clone()]);
        }
        let mut pending = vec![entry.path.clone()];
        let mut files = Vec::new();
        if let Some(client) = &self.remote {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            while let Some(directory) = pending.pop() {
                for child in runtime
                    .block_on(remote_listing(client, &directory))?
                    .entries
                {
                    if child.kind == "directory" {
                        pending.push(child.path);
                    } else {
                        files.push(child);
                    }
                }
            }
        } else {
            let root = self
                .root
                .as_ref()
                .ok_or_else(|| "Choose a location first".to_string())?;
            while let Some(relative) = pending.pop() {
                let directory = resolve_path(root, &relative)?;
                for child in list_directory(root, &directory)? {
                    if child.kind == "directory" {
                        pending.push(child.path);
                    } else {
                        files.push(child);
                    }
                }
            }
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    fn active_location(&self) -> String {
        self.root
            .as_ref()
            .map(|root| root.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.remote_id.clone())
    }

    fn create_folder(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains(['/', '\\'])
            || Path::new(name).components().count() != 1
        {
            return Err("Enter a valid folder name".into());
        }
        if let Some(client) = &self.remote {
            let target = join_remote_path(&self.current, name);
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            runtime.block_on(client.create_dir(&target))?;
        } else {
            fs::create_dir(self.current_path()?.join(name)).map_err(|error| error.to_string())?;
        }
        self.refresh()
    }

    fn import_files(&mut self, paths: Vec<PathBuf>) -> Result<(), String> {
        if let Some(client) = &self.remote {
            let directory = normalize_remote_relative_path(&self.current)?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            for path in paths {
                let source = fs::canonicalize(path).map_err(|error| error.to_string())?;
                if !source.is_file() {
                    return Err("Only files can be added to a WebDAV location".into());
                }
                let name = source
                    .file_name()
                    .ok_or_else(|| "The selected file has no name".to_string())?
                    .to_string_lossy()
                    .into_owned();
                let mut suffix = 1;
                let target = loop {
                    let candidate =
                        join_remote_path(&directory, &super::suffixed_name(&name, suffix));
                    if !runtime.block_on(client.exists(&candidate))? {
                        break candidate;
                    }
                    suffix += 1;
                    if suffix > 999 {
                        return Err("Could not find an available destination name".into());
                    }
                };
                let bytes = fs::read(source).map_err(|error| error.to_string())?;
                runtime.block_on(client.write_file(&target, bytes))?;
            }
        } else {
            let destination = self.current_path()?;
            for source in paths {
                let source = fs::canonicalize(source).map_err(|error| error.to_string())?;
                let Some(name) = source.file_name() else {
                    continue;
                };
                let target = available_destination(&destination, &name.to_string_lossy());
                super::copy_recursively(&source, &target)?;
            }
        }
        self.refresh()
    }

    fn preview_content(&self) -> Result<PreviewResult, String> {
        let entry = self
            .selected_entry()
            .ok_or_else(|| "Select a file first".to_string())?;
        if entry.kind == "directory" {
            return Err("Select a file instead of a folder".into());
        }
        let extension = entry.extension.to_ascii_lowercase();
        if matches!(entry.kind.as_str(), "video" | "audio" | "archive" | "file")
            || matches!(extension.as_str(), "pdf")
            || (entry.kind == "document" && !matches!(extension.as_str(), "docx" | "pptx" | "xlsx"))
        {
            return Ok(PreviewResult::External);
        }
        let max_bytes = match entry.kind.as_str() {
            "text" => 2 * 1024 * 1024,
            "document" => 16 * 1024 * 1024,
            "image" => 64 * 1024 * 1024,
            _ => 0,
        };
        if max_bytes == 0 {
            return Err("Use Open externally for this file type".into());
        }
        let bytes = if let Some(client) = &self.remote {
            let path = normalize_remote_relative_path(&entry.path)?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            let metadata = runtime.block_on(client.stat(&path))?;
            if metadata.is_collection {
                return Err("Select a file to preview".into());
            }
            if metadata.size_bytes > max_bytes {
                return Err(format!(
                    "Preview is limited to {} MB",
                    max_bytes / 1024 / 1024
                ));
            }
            runtime.block_on(client.read_file(&path))?
        } else {
            let path = current_selection_absolute(self)?;
            let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
            if !metadata.is_file() {
                return Err("Select a file to preview".into());
            }
            if metadata.len() > max_bytes {
                return Err(format!(
                    "Preview is limited to {} MB",
                    max_bytes / 1024 / 1024
                ));
            }
            fs::read(path).map_err(|error| error.to_string())?
        };
        if bytes.len() as u64 > max_bytes {
            return Err(format!(
                "Preview is limited to {} MB",
                max_bytes / 1024 / 1024
            ));
        }
        if entry.kind == "image" {
            if extension == "svg" {
                let image = Image::load_from_svg_data(&bytes).map_err(|error| error.to_string())?;
                return Ok(PreviewResult::Image(image));
            }
            let rgba = if extension == "avif" {
                decode_avif_rgba(&bytes).ok_or_else(|| "Could not decode AVIF image".to_string())?
            } else {
                let dimensions = image::ImageReader::new(Cursor::new(&bytes))
                    .with_guessed_format()
                    .map_err(|error| error.to_string())?
                    .into_dimensions()
                    .map_err(|error| error.to_string())?;
                if dimensions.0 > 8192 || dimensions.1 > 8192 {
                    return Err("Image preview is limited to 8192 pixels per dimension".into());
                }
                image::load_from_memory(&bytes)
                    .map_err(|error| error.to_string())?
                    .to_rgba8()
            };
            if rgba.width() > 8192 || rgba.height() > 8192 {
                return Err("Image preview is limited to 8192 pixels per dimension".into());
            }
            return Ok(PreviewResult::Image(rgba_to_slint_image(&rgba)));
        }
        let text = match extension.as_str() {
            "docx" => extract_docx(&bytes)?,
            "pptx" => extract_pptx(&bytes)?,
            "xlsx" => extract_xlsx(&bytes)?,
            "json" => serde_json::from_slice::<serde_json::Value>(&bytes)
                .map(|value| serde_json::to_string_pretty(&value).unwrap_or_default())
                .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned()),
            "md" | "markdown" => {
                markdown_plain_text(&String::from_utf8(bytes).map_err(|error| error.to_string())?)
            }
            "csv" => tabular_text(
                &String::from_utf8(bytes).map_err(|error| error.to_string())?,
                ',',
            ),
            "tsv" => tabular_text(
                &String::from_utf8(bytes).map_err(|error| error.to_string())?,
                '\t',
            ),
            _ => String::from_utf8(bytes).map_err(|error| error.to_string())?,
        };
        Ok(PreviewResult::Text(text))
    }

    fn open_selected(&mut self) -> Result<(), String> {
        let entry = self
            .selected_entry()
            .cloned()
            .ok_or_else(|| "Select a file first".to_string())?;
        if entry.kind == "directory" {
            return Err("Select a file instead of a folder".into());
        }
        if let Some(client) = &self.remote {
            let bytes = {
                let path = normalize_remote_relative_path(&entry.path)?;
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())?;
                runtime.block_on(client.read_file(&path))?
            };
            let suffix = Path::new(&entry.name)
                .extension()
                .and_then(|value| value.to_str())
                .map(|extension| format!(".{extension}"))
                .unwrap_or_default();
            let mut file = tempfile::Builder::new()
                .prefix("r2-list-")
                .suffix(&suffix)
                .tempfile()
                .map_err(|error| error.to_string())?;
            std::io::Write::write_all(&mut file, &bytes).map_err(|error| error.to_string())?;
            let path = file.into_temp_path();
            open::that(&path).map_err(|error| error.to_string())?;
            self.opened_temp_files.push(path);
        } else {
            open::that(current_selection_absolute(self)?).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn delete_selected(&mut self) -> Result<(), String> {
        let selected = self
            .selected
            .clone()
            .ok_or_else(|| "Select an item first".to_string())?;
        if let Some(client) = &self.remote {
            let path = normalize_remote_relative_path(&selected)?;
            if path.is_empty() {
                return Err("Cannot delete the root directory".into());
            }
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            runtime.block_on(client.remove(&path))?;
        } else {
            let path = current_selection_absolute(self)?;
            if path.is_dir() {
                fs::remove_dir_all(path).map_err(|error| error.to_string())?;
            } else {
                fs::remove_file(path).map_err(|error| error.to_string())?;
            }
        }
        if self
            .transfer
            .as_ref()
            .is_some_and(|(source, _, _)| source == &selected)
        {
            self.transfer = None;
        }
        self.selected = None;
        self.refresh()
    }
}

fn read_zip_entry(bytes: &[u8], name: &str) -> Result<String, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let mut file = archive.by_name(name).map_err(|error| error.to_string())?;
    if file.size() > MAX_OFFICE_XML_BYTES {
        return Err("Office preview content exceeds the 32 MB limit".into());
    }
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|error| error.to_string())?;
    Ok(contents)
}

fn xml_local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn xml_grouped_text(xml: &str, group_tag: &str, text_tag: &str) -> Result<Vec<String>, String> {
    let mut reader = XmlReader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut in_group = false;
    let mut in_text = false;
    let mut current = String::new();
    let mut values = Vec::new();
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(event) if xml_local_name(event.name().as_ref()) == group_tag => {
                in_group = true;
                current.clear();
            }
            Event::Start(event) if xml_local_name(event.name().as_ref()) == text_tag => {
                in_text = true;
            }
            Event::Text(event) if in_group && in_text => {
                current.push_str(event.as_ref());
            }
            Event::GeneralRef(event) if in_group && in_text => {
                if let Some(character) = event
                    .resolve_char_ref()
                    .map_err(|error| error.to_string())?
                {
                    current.push(character);
                } else if let Some(value) =
                    quick_xml::escape::resolve_predefined_entity(event.as_ref())
                {
                    current.push_str(value);
                } else {
                    return Err(format!("Unsupported XML entity &{};", event.as_ref()));
                }
            }
            Event::End(event) if xml_local_name(event.name().as_ref()) == text_tag => {
                in_text = false;
            }
            Event::End(event) if xml_local_name(event.name().as_ref()) == group_tag => {
                values.push(current.clone());
                current.clear();
                in_group = false;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(values)
}

fn extract_docx(bytes: &[u8]) -> Result<String, String> {
    let xml = read_zip_entry(bytes, "word/document.xml")?;
    Ok(xml_grouped_text(&xml, "p", "t")?.join("\n\n"))
}

fn extract_pptx(bytes: &[u8]) -> Result<String, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let mut slide_names = archive
        .file_names()
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    slide_names.sort_by_key(|name| {
        name.rsplit("slide")
            .next()
            .and_then(|part| part.strip_suffix(".xml"))
            .and_then(|number| number.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });
    let mut slides = Vec::new();
    for (index, name) in slide_names.iter().enumerate() {
        let mut file = archive.by_name(name).map_err(|error| error.to_string())?;
        if file.size() > MAX_OFFICE_XML_BYTES {
            return Err("Office preview content exceeds the 32 MB limit".into());
        }
        let mut xml = String::new();
        file.read_to_string(&mut xml)
            .map_err(|error| error.to_string())?;
        let text = xml_grouped_text(&xml, "p", "t")?.join("\n");
        slides.push(format!("Slide {}\n{text}", index + 1));
    }
    Ok(slides.join("\n\n"))
}

fn extract_xlsx(bytes: &[u8]) -> Result<String, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let shared_strings = match archive.by_name("xl/sharedStrings.xml") {
        Ok(mut file) => {
            if file.size() > MAX_OFFICE_XML_BYTES {
                return Err("Office preview content exceeds the 32 MB limit".into());
            }
            let mut xml = String::new();
            file.read_to_string(&mut xml)
                .map_err(|error| error.to_string())?;
            xml_grouped_text(&xml, "si", "t")?
        }
        Err(_) => Vec::new(),
    };
    let mut names = archive
        .file_names()
        .filter(|name| name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    names.sort_by_key(|name| {
        name.rsplit("sheet")
            .next()
            .and_then(|part| part.strip_suffix(".xml"))
            .and_then(|number| number.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });
    let mut output = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let mut file = archive.by_name(name).map_err(|error| error.to_string())?;
        if file.size() > MAX_OFFICE_XML_BYTES {
            return Err("Office preview content exceeds the 32 MB limit".into());
        }
        let mut xml = String::new();
        file.read_to_string(&mut xml)
            .map_err(|error| error.to_string())?;
        let mut reader = XmlReader::from_str(&xml);
        reader.config_mut().trim_text(true);
        let mut rows = Vec::new();
        let mut row = Vec::new();
        let mut cell_type = String::new();
        let mut cell_text = String::new();
        let mut in_cell = false;
        let mut in_value = false;
        loop {
            match reader.read_event().map_err(|error| error.to_string())? {
                Event::Start(event) if xml_local_name(event.name().as_ref()) == "row" => {
                    row.clear()
                }
                Event::Start(event) if xml_local_name(event.name().as_ref()) == "c" => {
                    in_cell = true;
                    cell_type.clear();
                    cell_text.clear();
                    for attribute in event.attributes().flatten() {
                        if attribute.key.as_ref() == "t" {
                            cell_type = attribute.value.to_string();
                        }
                    }
                }
                Event::Start(event)
                    if matches!(xml_local_name(event.name().as_ref()), "v" | "t") =>
                {
                    in_value = true;
                }
                Event::Text(event) if in_value => {
                    let decoded = event.as_ref();
                    let decoded =
                        quick_xml::escape::unescape(&decoded).map_err(|error| error.to_string())?;
                    cell_text.push_str(&decoded);
                }
                Event::End(event) if matches!(xml_local_name(event.name().as_ref()), "v" | "t") => {
                    in_value = false;
                }
                Event::End(event) if xml_local_name(event.name().as_ref()) == "c" => {
                    if in_cell {
                        if cell_type == "s" {
                            let index = cell_text
                                .parse::<usize>()
                                .map_err(|error| error.to_string())?;
                            row.push(
                                shared_strings
                                    .get(index)
                                    .cloned()
                                    .ok_or_else(|| "Invalid shared string reference".to_string())?,
                            );
                        } else {
                            row.push(cell_text.clone());
                        }
                    }
                    in_cell = false;
                }
                Event::End(event) if xml_local_name(event.name().as_ref()) == "row" => {
                    rows.push(row.join("\t"));
                    row.clear();
                }
                Event::Eof => break,
                _ => {}
            }
        }
        output.push(format!("Sheet {}\n{}", index + 1, rows.join("\n")));
    }
    Ok(output.join("\n\n"))
}

thread_local! {
    static THUMBNAIL_CACHE: std::cell::RefCell<std::collections::HashMap<String, Option<Image>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn rgba_to_slint_image(image: &image::RgbaImage) -> Image {
    let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        image.as_raw(),
        image.width(),
        image.height(),
    );
    Image::from_rgba8(pixels)
}

fn decode_avif_rgba(bytes: &[u8]) -> Option<image::RgbaImage> {
    use avif_decode::Image as AvifImage;
    let decoded = avif_decode::Decoder::from_avif(bytes).ok()?.to_image().ok()?;
    let (width, height, data): (u32, u32, Vec<u8>) = match decoded {
        AvifImage::Rgba8(image) => {
            let mut data = Vec::with_capacity(image.width() * image.height() * 4);
            for pixel in image.buf() {
                data.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
            }
            (image.width() as u32, image.height() as u32, data)
        }
        AvifImage::Rgb8(image) => {
            let mut data = Vec::with_capacity(image.width() * image.height() * 4);
            for pixel in image.buf() {
                data.extend_from_slice(&[pixel.r, pixel.g, pixel.b, 255]);
            }
            (image.width() as u32, image.height() as u32, data)
        }
        AvifImage::Gray8(image) => {
            let mut data = Vec::with_capacity(image.width() * image.height() * 4);
            for pixel in image.buf() {
                let value = pixel.value();
                data.extend_from_slice(&[value, value, value, 255]);
            }
            (image.width() as u32, image.height() as u32, data)
        }
        AvifImage::Rgba16(image) => {
            let mut data = Vec::with_capacity(image.width() * image.height() * 4);
            for pixel in image.buf() {
                data.extend_from_slice(&[
                    (pixel.r >> 8) as u8,
                    (pixel.g >> 8) as u8,
                    (pixel.b >> 8) as u8,
                    (pixel.a >> 8) as u8,
                ]);
            }
            (image.width() as u32, image.height() as u32, data)
        }
        AvifImage::Rgb16(image) => {
            let mut data = Vec::with_capacity(image.width() * image.height() * 4);
            for pixel in image.buf() {
                data.extend_from_slice(&[
                    (pixel.r >> 8) as u8,
                    (pixel.g >> 8) as u8,
                    (pixel.b >> 8) as u8,
                    255,
                ]);
            }
            (image.width() as u32, image.height() as u32, data)
        }
        AvifImage::Gray16(image) => {
            let mut data = Vec::with_capacity(image.width() * image.height() * 4);
            for pixel in image.buf() {
                let value = (pixel.value() >> 8) as u8;
                data.extend_from_slice(&[value, value, value, 255]);
            }
            (image.width() as u32, image.height() as u32, data)
        }
    };
    image::RgbaImage::from_raw(width, height, data)
}

fn thumbnail_from_bytes(bytes: &[u8], extension: &str) -> Option<Image> {
    if extension.eq_ignore_ascii_case("svg") {
        return Image::load_from_svg_data(bytes).ok();
    }
    let thumbnail = if extension.eq_ignore_ascii_case("avif") {
        let decoded = decode_avif_rgba(bytes)?;
        if decoded.width() > 4096 || decoded.height() > 4096 {
            return None;
        }
        image::DynamicImage::ImageRgba8(decoded)
            .thumbnail(160, 90)
            .to_rgba8()
    } else {
        let dimensions = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()?;
        if dimensions.0 > 4096 || dimensions.1 > 4096 {
            return None;
        }
        image::load_from_memory(bytes)
            .ok()?
            .thumbnail(160, 90)
            .to_rgba8()
    };
    Some(rgba_to_slint_image(&thumbnail))
}

fn local_thumbnail(state: &AppState, entry: &DirectoryEntry) -> Option<Image> {
    if entry.kind != "image" || entry.size_bytes > 8 * 1024 * 1024 {
        return None;
    }
    let path = resolve_path(state.root.as_ref()?, &entry.path).ok()?;
    let bytes = fs::read(path).ok()?;
    thumbnail_from_bytes(&bytes, &entry.extension)
}

fn remote_thumbnail(state: &AppState, entry: &DirectoryEntry) -> Option<Image> {
    if entry.kind != "image" || entry.size_bytes > 8 * 1024 * 1024 {
        return None;
    }
    let client = state.remote.as_ref()?;
    let path = normalize_remote_relative_path(&entry.path).ok()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .ok()?;
    let bytes = runtime.block_on(client.read_file(&path)).ok()?;
    thumbnail_from_bytes(&bytes, &entry.extension)
}

fn cached_thumbnail(state: &AppState, entry: &DirectoryEntry) -> Option<Image> {
    let key = format!(
        "{}:{}:{}:{}",
        state.remote_id, entry.path, entry.size_bytes, entry.modified_ms
    );
    if let Some(cached) = THUMBNAIL_CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return cached;
    }
    let thumbnail = if state.remote.is_some() {
        remote_thumbnail(state, entry)
    } else {
        local_thumbnail(state, entry)
    };
    THUMBNAIL_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 300 {
            cache.clear();
        }
        cache.insert(key, thumbnail.clone());
    });
    thumbnail
}

fn is_ignored_entry(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with('.')
        || matches!(
            lower.as_str(),
            "desktop.ini"
                | "thumbs.db"
                | "ehthumbs.db"
                | "ntuser.dat"
                | "ntuser.ini"
                | "pagefile.sys"
                | "hiberfil.sys"
                | "swapfile.sys"
                | "dumpstack.log.tmp"
                | "$recycle.bin"
                | "system volume information"
                | "lost+found"
        )
}

fn visible_entries(
    state: &AppState,
    search: &str,
    include_thumbnails: bool,
    ascending: bool,
) -> Vec<FileRow> {
    let locale = state
        .config
        .locale
        .clone()
        .unwrap_or_else(|| system_locale().to_owned());
    let mut entries: Vec<_> = state
        .entries
        .iter()
        .filter(|entry| {
            entry.name.to_lowercase().contains(&search.to_lowercase())
                && (state.config.show_hidden || !is_ignored_entry(&entry.name))
        })
        .collect();
    entries.sort_by(|left, right| {
        let directory_order = (right.kind == "directory").cmp(&(left.kind == "directory"));
        if directory_order != std::cmp::Ordering::Equal {
            return directory_order;
        }
        let name_order = left.name.to_lowercase().cmp(&right.name.to_lowercase());
        if ascending {
            name_order
        } else {
            name_order.reverse()
        }
    });
    let mut thumbnail_budget = 32;
    entries
        .into_iter()
        .map(|entry| {
            let thumbnail = if include_thumbnails && entry.kind == "image" && thumbnail_budget > 0 {
                thumbnail_budget -= 1;
                cached_thumbnail(state, entry)
            } else {
                None
            };
            FileRow {
                name: entry.name.clone().into(),
                path: entry.path.clone().into(),
                kind: entry.kind.clone().into(),
                size: if entry.kind == "directory" {
                    "—".into()
                } else {
                    format_bytes(entry.size_bytes).into()
                },
                modified: format_modified(entry.modified_ms, &locale).into(),
                is_directory: entry.kind == "directory",
                has_thumbnail: thumbnail.is_some(),
                thumbnail: thumbnail.unwrap_or_default(),
            }
        })
        .collect()
}

fn sibling_folders(state: &AppState) -> Vec<SiblingFolder> {
    let list_path = match state.current.rsplit_once('/') {
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
    };
    let entries = if let Some(client) = &state.remote {
        let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        else {
            return Vec::new();
        };
        match runtime.block_on(remote_listing(client, &list_path)) {
            Ok(listing) => listing.entries,
            Err(_) => return Vec::new(),
        }
    } else if let Some(root) = &state.root {
        match resolve_path(root, &list_path).and_then(|path| list_directory(root, &path)) {
            Ok(entries) => entries,
            Err(_) => return Vec::new(),
        }
    } else {
        return Vec::new();
    };
    let mut directories: Vec<_> = entries
        .into_iter()
        .filter(|entry| entry.kind == "directory")
        .collect();
    directories.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    directories
        .into_iter()
        .map(|entry| SiblingFolder {
            is_current: entry.path == state.current,
            name: entry.name.into(),
            path: entry.path.into(),
        })
        .collect()
}

fn update_window(window: &MainWindow, state: &AppState) {
    let locale = state
        .config
        .locale
        .clone()
        .unwrap_or_else(|| system_locale().to_owned());
    let search = window.get_search().to_string();
    let rows = visible_entries(
        state,
        &search,
        window.get_thumbnails(),
        window.get_sort_ascending(),
    );
    let row_count = rows.len();
    let previous_selection = window.get_selected_path();
    let current_selection = state
        .selected_entry()
        .map(|entry| entry.path.as_str())
        .unwrap_or_default();
    if previous_selection != current_selection {
        window.set_preview_expanded(false);
        window.set_preview_text(SharedString::default());
        window.set_preview_image_visible(false);
        if !current_selection.is_empty() {
            update_preview(window, state, &locale);
        }
    }
    let same_files = {
        let existing = window.get_files();
        existing.row_count() == rows.len()
            && rows.iter().enumerate().all(|(index, row)| {
                existing.row_data(index).is_some_and(|current| {
                    current.path == row.path
                        && current.name == row.name
                        && current.size == row.size
                        && current.modified == row.modified
                        && current.is_directory == row.is_directory
                        && current.has_thumbnail == row.has_thumbnail
                })
            })
    };
    if !same_files {
        window.set_files(ModelRc::new(VecModel::from(rows)));
    }
    let locations = state
        .config
        .directories
        .iter()
        .map(|location| LocationRow {
            path: location.path.clone().into(),
            label: location_display_name(location).into(),
            kind: location.kind.clone().into(),
            is_active: location.path == state.active_location(),
        })
        .collect::<Vec<_>>();
    window.set_locations(ModelRc::new(VecModel::from(locations)));
    let mut breadcrumb_path = String::new();
    let breadcrumbs = state
        .current
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            breadcrumb_path = if breadcrumb_path.is_empty() {
                segment.to_string()
            } else {
                format!("{breadcrumb_path}/{segment}")
            };
            BreadcrumbRow {
                label: segment.into(),
                path: breadcrumb_path.clone().into(),
            }
        })
        .collect::<Vec<_>>();
    window.set_breadcrumbs(ModelRc::new(VecModel::from(breadcrumbs)));
    let current = if state.current.is_empty() {
        state.active_location()
    } else if let Some(root) = &state.root {
        root.join(&state.current).display().to_string()
    } else {
        format!("{}:{}", state.active_location(), state.current)
    };
    let active_label = state
        .config
        .directories
        .iter()
        .find(|location| location.path == state.active_location())
        .map(location_display_name)
        .unwrap_or(current);
    let current_name = if state.current.is_empty() {
        active_label.clone()
    } else {
        Path::new(&state.current)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| state.current.clone())
    };
    window.set_root_path(state.active_location().into());
    window.set_current_path(state.current.clone().into());
    window.set_current_location(active_label.clone().into());
    window.set_current_name(current_name.into());
    window.set_connection_label(if state.remote.is_some() {
        "WEBDAV".into()
    } else {
        "LOCAL".into()
    });
    let entry_count = match locale.as_str() {
        "zh-CN" => format!("{row_count} 个项目"),
        "zh-TW" => format!("{row_count} 個項目"),
        "ja" => format!("{row_count} 項目"),
        "ko" => format!("{row_count}개 항목"),
        "fr" => {
            if row_count == 1 {
                "1 élément".to_string()
            } else {
                format!("{row_count} éléments")
            }
        }
        _ => format!(
            "{} item{}",
            row_count,
            if row_count == 1 { "" } else { "s" }
        ),
    };
    window.set_entry_count(entry_count.into());
    window.set_entry_total(row_count.to_string().into());
    window.set_transfer_pending(state.transfer.is_some());
    let transfer_message = state
        .transfer
        .as_ref()
        .map(|(path, is_move, _)| {
            let name = path.rsplit(['/', '\\']).next().unwrap_or_default();
            let template = if *is_move {
                translate(
                    &locale,
                    "Selected “{name}” for moving. Choose a destination and paste.",
                )
            } else {
                translate(
                    &locale,
                    "Copied “{name}”. Choose a destination and paste.",
                )
            };
            template.replace("{name}", name)
        })
        .unwrap_or_default();
    window.set_transfer_message(transfer_message.into());
    window.set_transfer_name(
        state
            .transfer
            .as_ref()
            .and_then(|(path, _, _)| path.rsplit(['/', '\\']).next())
            .unwrap_or_default()
            .into(),
    );
    if let Some(entry) = state.selected_entry() {
        let url = if entry.kind == "directory" {
            String::new()
        } else {
            build_object_url(&entry.path, &state.rules())
                .unwrap_or_else(|error| format!("Invalid URL rules: {error}"))
        };
        window.set_selected_name(entry.name.clone().into());
        window.set_selected_path(entry.path.clone().into());
        window.set_selected_url(url.into());
        window.set_selected_kind(entry.kind.clone().into());
        window.set_selected_extension(
            Path::new(&entry.name)
                .extension()
                .map(|extension| extension.to_string_lossy().to_uppercase())
                .unwrap_or_else(|| {
                    if entry.kind == "directory" {
                        "FOLDER".to_string()
                    } else {
                        "FILE".to_string()
                    }
                })
                .into(),
        );
        window.set_selected_modified(format_modified(entry.modified_ms, &locale).into());
        window.set_selected_size(
            if entry.kind == "directory" {
                "—".to_string()
            } else {
                format_bytes(entry.size_bytes)
            }
            .into(),
        );
        let parent = Path::new(&entry.path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_else(|| active_label.clone());
        window.set_selected_parent(parent.into());
        window.set_has_selection(true);
    } else {
        window.set_selected_name(SharedString::default());
        window.set_selected_path(SharedString::default());
        window.set_selected_url(SharedString::default());
        window.set_selected_kind(SharedString::default());
        window.set_selected_extension(SharedString::default());
        window.set_selected_modified(SharedString::default());
        window.set_selected_size(SharedString::default());
        window.set_selected_parent(SharedString::default());
        window.set_has_selection(false);
    }
}

fn localize_update_result(locale: &str, status: &str) -> (&'static str, String) {
    if let Some(latest) = status.strip_prefix("Update available: ") {
        return (
            "update",
            format!("{}: {latest}", translate(locale, "Update available")),
        );
    }
    if let Some(latest) = status
        .strip_prefix("Up to date (latest: ")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return (
            "ok",
            format!("{} · {latest}", translate(locale, "Up to date")),
        );
    }
    ("ok", status.to_string())
}

fn preview_error_text(locale: &str, error: &str) -> String {
    let known = match error {
        "Select a file instead of a folder" => Some("Select a file instead of a folder"),
        "Select a file first" => Some("Select a file first"),
        "Select a file to preview" => Some("Select a file to preview"),
        "Use Open externally for this file type" => Some("Use Open externally for this file type"),
        _ => None,
    };
    match known {
        Some(key) => translate(locale, key).to_string(),
        None => error.to_string(),
    }
}

fn update_preview(window: &MainWindow, state: &AppState, locale: &str) {
    match state.preview_content() {
        Ok(PreviewResult::Text(text)) => {
            window.set_preview_text(text.into());
            window.set_preview_image_visible(false);
        }
        Ok(PreviewResult::Image(image)) => {
            window.set_preview_text(SharedString::default());
            window.set_preview_image(image);
            window.set_preview_image_visible(true);
        }
        Ok(PreviewResult::External) => {
            window.set_preview_text(
                translate(
                    locale,
                    "No embedded preview is available. Use Open to launch this file.",
                )
                .into(),
            );
            window.set_preview_image_visible(false);
        }
        Err(error) => {
            window.set_preview_text(
                format!(
                    "{}: {}",
                    translate(locale, "Preview unavailable"),
                    preview_error_text(locale, &error)
                )
                .into(),
            );
            window.set_preview_image_visible(false);
        }
    }
}

fn update_location_settings(window: &MainWindow, state: &AppState) {
    let path = state
        .editing_location
        .clone()
        .unwrap_or_else(|| state.active_location());
    let location = state
        .config
        .directories
        .iter()
        .find(|directory| directory.path == path)
        .cloned();
    let rules = location
        .as_ref()
        .map(|directory| directory.rules.clone())
        .unwrap_or_default();
    let preview = object_url_preview(state, &rules);
    window.set_base_url(rules.base_url.into());
    window.set_prefix(rules.prefix.into());
    window.set_bucket(rules.bucket.unwrap_or_default().into());
    window.set_template(rules.template.unwrap_or_default().into());
    window.set_location_label(
        location
            .as_ref()
            .map(location_display_name)
            .unwrap_or_default()
            .into(),
    );
    window.set_location_kind(
        location
            .as_ref()
            .map(|directory| directory.kind.clone())
            .unwrap_or_else(|| "local".into())
            .into(),
    );
    window.set_location_endpoint(
        location
            .as_ref()
            .map(|directory| directory.endpoint.clone())
            .unwrap_or_default()
            .into(),
    );
    window.set_location_username(
        location
            .as_ref()
            .map(|directory| directory.username.clone())
            .unwrap_or_default()
            .into(),
    );
    window.set_location_password(
        location
            .as_ref()
            .map(|directory| directory.password.clone())
            .unwrap_or_default()
            .into(),
    );
    window.set_location_preview(preview.into());
}

fn object_url_preview(state: &AppState, rules: &UrlRulesConfig) -> String {
    let sample = state
        .selected_entry()
        .filter(|entry| entry.kind != "directory")
        .map(|entry| entry.path.as_str())
        .unwrap_or("folder/file.jpg");
    build_object_url(sample, rules).unwrap_or_else(|error| format!("Invalid URL rules: {error}"))
}

fn show_error(window: &MainWindow, error: impl std::fmt::Display) {
    window.set_status(format!("Error: {error}").into());
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn format_modified(modified_ms: u64, locale: &str) -> String {
    if modified_ms == 0 {
        return String::new();
    }
    i64::try_from(modified_ms)
        .ok()
        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
        .map(|date| {
            let date = date.with_timezone(&chrono::Local);
            let format = match locale {
                "zh-CN" | "zh-TW" => "%Y年%-m月%-d日 %H:%M",
                "ja" => "%Y年%-m月%-d日 %H:%M",
                "ko" => "%Y. %-m. %-d. %H:%M",
                "fr" => "%d/%m/%Y %H:%M",
                _ => "%b %-d, %Y %H:%M",
            };
            date.format(format).to_string()
        })
        .unwrap_or_default()
}

fn markdown_plain_text(markdown: &str) -> String {
    markdown
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                return None;
            }
            let mut line = trimmed
                .trim_start_matches('#')
                .trim_start_matches('>')
                .trim()
                .to_string();
            for prefix in ["- ", "* ", "+ ", "• "] {
                if let Some(content) = line.strip_prefix(prefix) {
                    line = format!("• {content}");
                    break;
                }
            }
            line = replace_markdown_links(&line);
            for marker in ["**", "__", "~~", "*", "_", "`"] {
                line = line.replace(marker, "");
            }
            Some(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn replace_markdown_links(line: &str) -> String {
    let mut output = String::new();
    let mut rest = line;
    while let Some(open_label) = rest.find('[') {
        output.push_str(&rest[..open_label]);
        let label_start = open_label + 1;
        let Some(close_label_offset) = rest[label_start..].find(']') else {
            output.push_str(&rest[open_label..]);
            return output;
        };
        let close_label = label_start + close_label_offset;
        if rest[close_label..].starts_with("](") {
            if let Some(close_url_offset) = rest[close_label + 2..].find(')') {
                output.push_str(&rest[label_start..close_label]);
                rest = &rest[close_label + 2 + close_url_offset + 1..];
                continue;
            }
        }
        output.push_str(&rest[open_label..=close_label]);
        rest = &rest[close_label + 1..];
    }
    output.push_str(rest);
    output
}

fn tabular_text(contents: &str, delimiter: char) -> String {
    let mut rows: Vec<Vec<String>> = vec![Vec::new()];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = contents.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            value if value == delimiter && !quoted => {
                rows.last_mut()
                    .expect("a row always exists")
                    .push(std::mem::take(&mut field));
            }
            '\n' if !quoted => {
                rows.last_mut()
                    .expect("a row always exists")
                    .push(std::mem::take(&mut field));
                if rows.len() >= 200 {
                    break;
                }
                rows.push(Vec::new());
            }
            '\r' if !quoted => {}
            value => field.push(value),
        }
    }
    if rows.len() < 200 || !field.is_empty() {
        rows.last_mut().expect("a row always exists").push(field);
    }
    rows.into_iter()
        .filter(|row| row.iter().any(|cell| !cell.is_empty()))
        .map(|row| row.join("  |  "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn locale_name(locale: &str) -> &'static str {
    match locale {
        "fr" => "Français",
        "zh-CN" => "简体中文",
        "zh-TW" => "繁體中文",
        "ja" => "日本語",
        "ko" => "한국어",
        _ => "English",
    }
}

fn translate(locale: &str, english: &'static str) -> &'static str {
    let translated = match locale {
        "zh-CN" => match english {
            "WORKSPACE" => "工作区",
            "OBJECT WORKSPACE" => "对象工作区",
            "Files" => "文件",
            "URL rules" => "URL 规则",
            "Settings" => "设置",
            "LOCATIONS" => "目录位置",
            "Add location" => "添加位置",
            "Local folder" => "本地文件夹",
            "WebDAV server" => "WebDAV 服务器",
            "URL mapping only" => "仅映射 URL",
            "Sync handled externally" => "同步由外部服务负责",
            "R2 sync is not performed by this app" => "本应用不执行 R2 同步",
            "LOCAL" => "本地",
            "Personal workspace" => "个人工作区",
            "LOCAL DIRECTORY" => "本地目录",
            "URL MAPPED" => "R2 URL 已映射",
            "Add files" => "添加文件",
            "Search files" => "搜索文件",
            "Search files and folders…" => "搜索文件或文件夹…",
            "New folder" => "新建文件夹",
            "NAME" => "名称",
            "SIZE" => "大小",
            "MODIFIED" => "修改时间",
            "This folder is empty" => "此文件夹为空",
            "No other folders" => "没有其他文件夹",
            "No matching files" => "没有匹配的文件",
            "Try another search term" => "请尝试其他搜索词",
            "Back to parent" => "返回上级",
            "Edit rules" => "编辑规则",
            "Edit location" => "编辑位置",
            "More actions" => "更多操作",
            "Added" => "添加时间",
            "Storage location" => "存储位置",
            "Public status" => "公开状态",
            "Configured" => "已配置",
            "Not configured" => "未配置",
            "Add files or create a folder to get started." => "添加文件或新建文件夹以开始使用。",
            "QUICK LOOK" => "快速预览",
            "Preview" => "预览",
            "Choose a file to preview" => "选择文件以预览",
            "No embedded preview is available. Use Open to launch this file." => {
                "此文件没有内嵌预览，请使用“打开”启动该文件。"
            }
            "Opened with the system default application." => "已使用系统默认应用打开。",
            "Preview unavailable" => "预览不可用",
            "R2 OBJECT URL" => "R2 对象 URL",
            "Copy URL" => "复制 URL",
            "Markdown" => "Markdown",
            "HTML" => "HTML 链接",
            "Open" => "打开",
            "Delete" => "删除",
            "Copy item" => "复制项目",
            "Move item" => "移动项目",
            "Paste here" => "粘贴到此处",
            "Copied “{name}”. Choose a destination and paste." => {
                "已复制“{name}”，请选择目标位置后粘贴。"
            }
            "Selected “{name}” for moving. Choose a destination and paste." => {
                "已选择“{name}”用于移动，请选择目标位置后粘贴。"
            }
            "Language" => "语言",
            "System language" => "跟随系统语言",
            "System" => "跟随系统",
            "Light" => "浅色",
            "Dark" => "深色",
            "Dark theme" => "深色主题",
            "Save" => "保存",
            "CONFIGURATION" => "配置文件",
            "Manage where configuration is stored, check for updates, and connect with GitHub." => {
                "管理配置文件保存位置、检查软件更新并连接 GitHub。"
            }
            "Version" => "版本",
            "Latest version" => "最新版本",
            "Open the project page or review the latest release on GitHub." => {
                "在 GitHub 上打开项目主页或查看最新版本。"
            }
            "Configuration folder" => "配置文件夹",
            "Settings file" => "设置文件",
            "R2 List saves its settings file in this folder." => {
                "R2 List 会将设置文件保存在此文件夹中。"
            }
            "Show hidden and system files" => "显示隐藏文件和系统文件",
            "Use default" => "使用默认",
            "Not checked" => "未检查",
            "Checking…" => "检查中…",
            "Checking GitHub for updates…" => "正在检查 GitHub 更新…",
            "Connected" => "已连接",
            "Connection failed" => "连接失败",
            "Update available" => "有可用更新",
            "Up to date" => "已是最新版本",
            "Update check failed" => "更新检查失败",
            "Choose folder" => "选择文件夹",
            "Reset" => "重置",
            "ABOUT & UPDATES" => "关于与更新",
            "Check for updates" => "检查更新",
            "Test GitHub connection" => "测试 GitHub 连接",
            "GitHub project" => "GitHub 项目",
            "Latest release" => "最新版本",
            "Location settings" => "位置设置",
            "Location name" => "位置名称",
            "Public base URL" => "公开基础 URL",
            "Object prefix (optional)" => "对象前缀（可选）",
            "Bucket (optional)" => "存储桶（可选）",
            "Template: {base}/{key}" => "模板：{base}/{key}",
            "Template placeholders: {base}, {key}, {path}, {bucket}" => {
                "模板占位符：{base}、{key}、{path}、{bucket}"
            }
            "Select a file instead of a folder" => "请选择文件而不是文件夹",
            "Select a file first" => "请先选择文件",
            "Select a file to preview" => "请选择要预览的文件",
            "Use Open externally for this file type" => "请使用“打开”在外部打开此文件类型",
            "Save location" => "保存位置",
            "Connect WebDAV" => "连接 WebDAV",
            "Create folder" => "创建文件夹",
            "Cancel" => "取消",
            "Close" => "关闭",
            "Folder name" => "文件夹名称",
            "Server address" => "服务器地址",
            "Username" => "用户名",
            "Password" => "密码",
            "Connect" => "连接",
            "Directory" => "文件夹",
            "Copy other formats" => "复制为其他格式",
            "Local storage" => "本地存储",
            _ => english,
        },
        "zh-TW" => match english {
            "WORKSPACE" => "工作區",
            "OBJECT WORKSPACE" => "物件工作區",
            "Files" => "檔案",
            "URL rules" => "URL 規則",
            "Settings" => "設定",
            "LOCATIONS" => "目錄位置",
            "Add location" => "新增位置",
            "Local folder" => "本機資料夾",
            "WebDAV server" => "WebDAV 伺服器",
            "URL mapping only" => "僅對應 URL",
            "Sync handled externally" => "同步由外部服務負責",
            "R2 sync is not performed by this app" => "此應用程式不會同步至 R2",
            "LOCAL" => "本機",
            "Personal workspace" => "個人工作區",
            "LOCAL DIRECTORY" => "本機目錄",
            "URL MAPPED" => "R2 URL 已對應",
            "Add files" => "新增檔案",
            "Search files" => "搜尋檔案",
            "Search files and folders…" => "搜尋檔案或資料夾…",
            "New folder" => "新增資料夾",
            "NAME" => "名稱",
            "SIZE" => "大小",
            "MODIFIED" => "修改時間",
            "This folder is empty" => "此資料夾是空的",
            "No other folders" => "沒有其他資料夾",
            "No matching files" => "沒有符合的檔案",
            "Try another search term" => "請嘗試其他搜尋詞",
            "Back to parent" => "返回上層",
            "Edit rules" => "編輯規則",
            "Edit location" => "編輯位置",
            "More actions" => "更多操作",
            "Added" => "新增時間",
            "Storage location" => "儲存位置",
            "Public status" => "公開狀態",
            "Configured" => "已設定",
            "Not configured" => "未設定",
            "Add files or create a folder to get started." => "新增檔案或資料夾以開始使用。",
            "QUICK LOOK" => "快速預覽",
            "Preview" => "預覽",
            "Choose a file to preview" => "選擇檔案以預覽",
            "No embedded preview is available. Use Open to launch this file." => {
                "此檔案沒有內嵌預覽，請使用「開啟」啟動該檔案。"
            }
            "Opened with the system default application." => "已使用系統預設應用程式開啟。",
            "Preview unavailable" => "無法預覽",
            "R2 OBJECT URL" => "R2 物件 URL",
            "Copy URL" => "複製 URL",
            "Markdown" => "Markdown",
            "HTML" => "HTML 連結",
            "Open" => "開啟",
            "Delete" => "刪除",
            "Copy item" => "複製項目",
            "Move item" => "移動項目",
            "Paste here" => "貼上至此處",
            "Copied “{name}”. Choose a destination and paste." => {
                "已複製「{name}」，請選擇目標位置後貼上。"
            }
            "Selected “{name}” for moving. Choose a destination and paste." => {
                "已選擇「{name}」用於移動，請選擇目標位置後貼上。"
            }
            "Language" => "語言",
            "System language" => "跟隨系統語言",
            "System" => "跟隨系統",
            "Light" => "淺色",
            "Dark" => "深色",
            "Dark theme" => "深色主題",
            "Save" => "儲存",
            "CONFIGURATION" => "設定檔",
            "Manage where configuration is stored, check for updates, and connect with GitHub." => {
                "管理設定檔儲存位置、檢查軟體更新並連線 GitHub。"
            }
            "Version" => "版本",
            "Latest version" => "最新版本",
            "Open the project page or review the latest release on GitHub." => {
                "在 GitHub 上開啟專案首頁或查看最新版本。"
            }
            "Configuration folder" => "設定資料夾",
            "Settings file" => "設定檔",
            "R2 List saves its settings file in this folder." => {
                "R2 List 會將設定檔儲存在此資料夾中。"
            }
            "Show hidden and system files" => "顯示隱藏檔案和系統檔案",
            "Use default" => "使用預設",
            "Not checked" => "未檢查",
            "Checking…" => "檢查中…",
            "Checking GitHub for updates…" => "正在檢查 GitHub 更新…",
            "Connected" => "已連線",
            "Connection failed" => "連線失敗",
            "Update available" => "有可用更新",
            "Up to date" => "已是最新版本",
            "Update check failed" => "更新檢查失敗",
            "Choose folder" => "選擇資料夾",
            "Reset" => "重設",
            "ABOUT & UPDATES" => "關於與更新",
            "Check for updates" => "檢查更新",
            "Test GitHub connection" => "測試 GitHub 連線",
            "GitHub project" => "GitHub 專案",
            "Latest release" => "最新版本",
            "Location settings" => "位置設定",
            "Location name" => "位置名稱",
            "Public base URL" => "公開基礎 URL",
            "Object prefix (optional)" => "物件前綴（選用）",
            "Bucket (optional)" => "儲存桶（選用）",
            "Template: {base}/{key}" => "範本：{base}/{key}",
            "Template placeholders: {base}, {key}, {path}, {bucket}" => {
                "範本預留位置：{base}、{key}、{path}、{bucket}"
            }
            "Select a file instead of a folder" => "請選擇檔案而不是資料夾",
            "Select a file first" => "請先選擇檔案",
            "Select a file to preview" => "請選擇要預覽的檔案",
            "Use Open externally for this file type" => "請使用「開啟」在外部開啟此檔案類型",
            "Save location" => "儲存位置",
            "Connect WebDAV" => "連線 WebDAV",
            "Create folder" => "建立資料夾",
            "Cancel" => "取消",
            "Close" => "關閉",
            "Folder name" => "資料夾名稱",
            "Server address" => "伺服器位址",
            "Username" => "使用者名稱",
            "Password" => "密碼",
            "Connect" => "連線",
            "Directory" => "資料夾",
            "Copy other formats" => "複製為其他格式",
            "Local storage" => "本機儲存",
            _ => english,
        },
        "ja" => match english {
            "WORKSPACE" => "ワークスペース",
            "OBJECT WORKSPACE" => "オブジェクトワークスペース",
            "Files" => "ファイル",
            "URL rules" => "URL ルール",
            "Settings" => "設定",
            "LOCATIONS" => "場所",
            "Add location" => "場所を追加",
            "Local folder" => "ローカルフォルダー",
            "WebDAV server" => "WebDAV サーバー",
            "URL mapping only" => "URL のマッピングのみ",
            "Sync handled externally" => "同期は外部サービスが実行",
            "R2 sync is not performed by this app" => "このアプリは R2 を同期しません",
            "LOCAL" => "ローカル",
            "Personal workspace" => "個人ワークスペース",
            "LOCAL DIRECTORY" => "ローカルディレクトリ",
            "URL MAPPED" => "R2 URL をマッピング済み",
            "Add files" => "ファイルを追加",
            "Search files" => "ファイルを検索",
            "Search files and folders…" => "ファイルやフォルダーを検索…",
            "New folder" => "新しいフォルダー",
            "NAME" => "名前",
            "SIZE" => "サイズ",
            "MODIFIED" => "更新日時",
            "This folder is empty" => "このフォルダーは空です",
            "No other folders" => "他のフォルダーはありません",
            "No matching files" => "一致するファイルがありません",
            "Try another search term" => "別の検索語をお試しください",
            "Back to parent" => "親フォルダーに戻る",
            "Edit rules" => "ルールを編集",
            "Edit location" => "場所を編集",
            "More actions" => "その他の操作",
            "Added" => "追加日時",
            "Storage location" => "保存場所",
            "Public status" => "公開状態",
            "Configured" => "設定済み",
            "Not configured" => "未設定",
            "Add files or create a folder to get started." => {
                "ファイルまたはフォルダーを追加してください。"
            }
            "QUICK LOOK" => "クイックプレビュー",
            "Preview" => "プレビュー",
            "Choose a file to preview" => "プレビューするファイルを選択",
            "No embedded preview is available. Use Open to launch this file." => {
                "このファイルには埋め込みプレビューがありません。「開く」で起動してください。"
            }
            "Opened with the system default application." => "システムの既定のアプリで開きました。",
            "Preview unavailable" => "プレビューを利用できません",
            "R2 OBJECT URL" => "R2 オブジェクト URL",
            "Copy URL" => "URL をコピー",
            "Markdown" => "Markdown",
            "HTML" => "HTML リンク",
            "Open" => "開く",
            "Delete" => "削除",
            "Copy item" => "項目をコピー",
            "Move item" => "項目を移動",
            "Paste here" => "ここに貼り付け",
            "Copied “{name}”. Choose a destination and paste." => {
                "「{name}」をコピーしました。貼り付け先を選択してください。"
            }
            "Selected “{name}” for moving. Choose a destination and paste." => {
                "「{name}」を移動用に選択しました。貼り付け先を選択してください。"
            }
            "Language" => "言語",
            "System" => "システムに従う",
            "System language" => "システム言語",
            "Light" => "ライト",
            "Dark" => "ダーク",
            "Dark theme" => "ダークテーマ",
            "Save" => "保存",
            "CONFIGURATION" => "設定ファイル",
            "Manage where configuration is stored, check for updates, and connect with GitHub." => {
                "設定ファイルの保存先の管理、更新の確認、GitHub への接続を行います。"
            }
            "Version" => "バージョン",
            "Latest version" => "最新バージョン",
            "Open the project page or review the latest release on GitHub." => {
                "GitHub でプロジェクトページを開くか、最新リリースを確認します。"
            }
            "Configuration folder" => "設定フォルダー",
            "Settings file" => "設定ファイル",
            "R2 List saves its settings file in this folder." => {
                "R2 List は設定ファイルをこのフォルダーに保存します。"
            }
            "Show hidden and system files" => "隠しファイルとシステムファイルを表示",
            "Use default" => "既定を使用",
            "Not checked" => "未確認",
            "Checking…" => "確認中…",
            "Checking GitHub for updates…" => "GitHub の更新を確認しています…",
            "Connected" => "接続済み",
            "Connection failed" => "接続に失敗しました",
            "Update available" => "更新があります",
            "Up to date" => "最新です",
            "Update check failed" => "更新の確認に失敗しました",
            "Choose folder" => "フォルダーを選択",
            "Reset" => "リセット",
            "ABOUT & UPDATES" => "情報と更新",
            "Check for updates" => "更新を確認",
            "Test GitHub connection" => "GitHub 接続をテスト",
            "GitHub project" => "GitHub プロジェクト",
            "Latest release" => "最新リリース",
            "Location settings" => "場所の設定",
            "Location name" => "場所の名前",
            "Public base URL" => "公開ベース URL",
            "Object prefix (optional)" => "オブジェクトプレフィックス（任意）",
            "Bucket (optional)" => "バケット（任意）",
            "Template: {base}/{key}" => "テンプレート: {base}/{key}",
            "Template placeholders: {base}, {key}, {path}, {bucket}" => {
                "テンプレートのプレースホルダー: {base}, {key}, {path}, {bucket}"
            }
            "Select a file instead of a folder" => "フォルダではなくファイルを選択してください",
            "Select a file first" => "先にファイルを選択してください",
            "Select a file to preview" => "プレビューするファイルを選択してください",
            "Use Open externally for this file type" => {
                "このファイルの種類は「開く」で外部アプリを使用してください"
            }
            "Save location" => "場所を保存",
            "Connect WebDAV" => "WebDAV に接続",
            "Create folder" => "フォルダーを作成",
            "Cancel" => "キャンセル",
            "Close" => "閉じる",
            "Folder name" => "フォルダー名",
            "Server address" => "サーバーアドレス",
            "Username" => "ユーザー名",
            "Password" => "パスワード",
            "Connect" => "接続",
            "Directory" => "フォルダー",
            "Copy other formats" => "他の形式でコピー",
            "Local storage" => "ローカルストレージ",
            _ => english,
        },
        "ko" => match english {
            "WORKSPACE" => "작업 공간",
            "OBJECT WORKSPACE" => "오브젝트 작업 공간",
            "Files" => "파일",
            "URL rules" => "URL 규칙",
            "Settings" => "설정",
            "LOCATIONS" => "위치",
            "Add location" => "위치 추가",
            "Local folder" => "로컬 폴더",
            "WebDAV server" => "WebDAV 서버",
            "URL mapping only" => "URL 매핑 전용",
            "Sync handled externally" => "동기화는 외부 서비스에서 처리",
            "R2 sync is not performed by this app" => "이 앱은 R2 동기화를 수행하지 않습니다",
            "LOCAL" => "로컬",
            "Personal workspace" => "개인 작업 공간",
            "LOCAL DIRECTORY" => "로컬 디렉터리",
            "URL MAPPED" => "R2 URL 매핑됨",
            "Add files" => "파일 추가",
            "Search files" => "파일 검색",
            "Search files and folders…" => "파일 및 폴더 검색…",
            "New folder" => "새 폴더",
            "NAME" => "이름",
            "SIZE" => "크기",
            "MODIFIED" => "수정 날짜",
            "This folder is empty" => "폴더가 비어 있습니다",
            "No other folders" => "다른 폴더가 없습니다",
            "No matching files" => "일치하는 파일이 없습니다",
            "Try another search term" => "다른 검색어를 입력해 보세요",
            "Back to parent" => "상위 폴더로 돌아가기",
            "Edit rules" => "규칙 편집",
            "Edit location" => "위치 편집",
            "More actions" => "추가 작업",
            "Added" => "추가한 날짜",
            "Storage location" => "저장 위치",
            "Public status" => "공개 상태",
            "Configured" => "구성됨",
            "Not configured" => "구성되지 않음",
            "Add files or create a folder to get started." => {
                "파일을 추가하거나 폴더를 만들어 시작하세요."
            }
            "QUICK LOOK" => "빠른 미리 보기",
            "Preview" => "미리 보기",
            "Choose a file to preview" => "미리 볼 파일을 선택하세요",
            "No embedded preview is available. Use Open to launch this file." => {
                "이 파일에는 내장 미리보기가 없습니다. '열기'로 실행하세요."
            }
            "Opened with the system default application." => "시스템 기본 앱으로 열었습니다.",
            "Preview unavailable" => "미리보기를 사용할 수 없습니다",
            "R2 OBJECT URL" => "R2 객체 URL",
            "Copy URL" => "URL 복사",
            "Markdown" => "Markdown",
            "HTML" => "HTML 링크",
            "Open" => "열기",
            "Delete" => "삭제",
            "Copy item" => "항목 복사",
            "Move item" => "항목 이동",
            "Paste here" => "여기에 붙여넣기",
            "Copied “{name}”. Choose a destination and paste." => {
                "“{name}”을(를) 복사했습니다. 대상 위치를 선택한 후 붙여넣으세요."
            }
            "Selected “{name}” for moving. Choose a destination and paste." => {
                "“{name}”을(를) 이동용으로 선택했습니다. 대상 위치를 선택한 후 붙여넣으세요."
            }
            "Language" => "언어",
            "System" => "시스템 설정",
            "System language" => "시스템 언어",
            "Light" => "라이트",
            "Dark" => "다크",
            "Dark theme" => "다크 테마",
            "Save" => "저장",
            "CONFIGURATION" => "구성",
            "Manage where configuration is stored, check for updates, and connect with GitHub." => {
                "구성 파일 저장 위치를 관리하고, 업데이트를 확인하고, GitHub에 연결합니다."
            }
            "Version" => "버전",
            "Latest version" => "최신 버전",
            "Open the project page or review the latest release on GitHub." => {
                "GitHub에서 프로젝트 페이지를 열거나 최신 릴리스를 확인합니다."
            }
            "Configuration folder" => "구성 폴더",
            "Settings file" => "설정 파일",
            "R2 List saves its settings file in this folder." => {
                "R2 List는 이 폴더에 설정 파일을 저장합니다."
            }
            "Show hidden and system files" => "숨김 및 시스템 파일 표시",
            "Use default" => "기본값 사용",
            "Not checked" => "확인 안 함",
            "Checking…" => "확인 중…",
            "Checking GitHub for updates…" => "GitHub 업데이트를 확인하는 중…",
            "Connected" => "연결됨",
            "Connection failed" => "연결 실패",
            "Update available" => "업데이트 사용 가능",
            "Up to date" => "최신 상태",
            "Update check failed" => "업데이트 확인 실패",
            "Choose folder" => "폴더 선택",
            "Reset" => "초기화",
            "ABOUT & UPDATES" => "정보 및 업데이트",
            "Check for updates" => "업데이트 확인",
            "Test GitHub connection" => "GitHub 연결 테스트",
            "GitHub project" => "GitHub 프로젝트",
            "Latest release" => "최신 릴리스",
            "Location settings" => "위치 설정",
            "Location name" => "위치 이름",
            "Public base URL" => "공개 기본 URL",
            "Object prefix (optional)" => "객체 접두사(선택 사항)",
            "Bucket (optional)" => "버킷(선택 사항)",
            "Template: {base}/{key}" => "템플릿: {base}/{key}",
            "Template placeholders: {base}, {key}, {path}, {bucket}" => {
                "템플릿 자리 표시자: {base}, {key}, {path}, {bucket}"
            }
            "Select a file instead of a folder" => "폴더 대신 파일을 선택하세요",
            "Select a file first" => "먼저 파일을 선택하세요",
            "Select a file to preview" => "미리 볼 파일을 선택하세요",
            "Use Open externally for this file type" => "이 파일 형식은 '열기'로 외부에서 여세요",
            "Save location" => "위치 저장",
            "Connect WebDAV" => "WebDAV 연결",
            "Create folder" => "폴더 만들기",
            "Cancel" => "취소",
            "Close" => "닫기",
            "Folder name" => "폴더 이름",
            "Server address" => "서버 주소",
            "Username" => "사용자 이름",
            "Password" => "암호",
            "Connect" => "연결",
            "Directory" => "폴더",
            "Copy other formats" => "다른 형식으로 복사",
            "Local storage" => "로컬 저장소",
            _ => english,
        },
        "fr" => match english {
            "WORKSPACE" => "ESPACE DE TRAVAIL",
            "OBJECT WORKSPACE" => "ESPACE OBJETS",
            "Files" => "Fichiers",
            "URL rules" => "Règles d’URL",
            "Settings" => "Paramètres",
            "LOCATIONS" => "EMPLACEMENTS",
            "Add location" => "Ajouter un emplacement",
            "Local folder" => "Dossier local",
            "WebDAV server" => "Serveur WebDAV",
            "URL mapping only" => "Mappage d’URL uniquement",
            "Sync handled externally" => "Synchronisation externe",
            "R2 sync is not performed by this app" => "Cette application ne synchronise pas R2",
            "LOCAL" => "LOCAL",
            "Personal workspace" => "Espace personnel",
            "LOCAL DIRECTORY" => "DOSSIER LOCAL",
            "URL MAPPED" => "URL R2 MAPPÉE",
            "Add files" => "Ajouter des fichiers",
            "Search files" => "Rechercher des fichiers",
            "Search files and folders…" => "Rechercher fichiers et dossiers…",
            "New folder" => "Nouveau dossier",
            "NAME" => "NOM",
            "SIZE" => "TAILLE",
            "MODIFIED" => "MODIFIÉ",
            "This folder is empty" => "Ce dossier est vide",
            "No other folders" => "Aucun autre dossier",
            "No matching files" => "Aucun fichier correspondant",
            "Try another search term" => "Essayez un autre terme de recherche",
            "Back to parent" => "Revenir au dossier parent",
            "Edit rules" => "Modifier les règles",
            "Edit location" => "Modifier l’emplacement",
            "More actions" => "Plus d’actions",
            "Added" => "Ajouté",
            "Storage location" => "Emplacement de stockage",
            "Public status" => "État public",
            "Configured" => "Configuré",
            "Not configured" => "Non configuré",
            "Add files or create a folder to get started." => {
                "Ajoutez des fichiers ou créez un dossier pour commencer."
            }
            "QUICK LOOK" => "APERÇU RAPIDE",
            "Preview" => "Aperçu",
            "Choose a file to preview" => "Choisissez un fichier à prévisualiser",
            "No embedded preview is available. Use Open to launch this file." => {
                "Aucun aperçu intégré n’est disponible. Utilisez Ouvrir pour lancer ce fichier."
            }
            "Opened with the system default application." => {
                "Ouvert avec l’application par défaut du système."
            }
            "Preview unavailable" => "Aperçu indisponible",
            "R2 OBJECT URL" => "URL DE L’OBJET R2",
            "Copy URL" => "Copier l’URL",
            "Markdown" => "Markdown",
            "HTML" => "Liens HTML",
            "Open" => "Ouvrir",
            "Delete" => "Supprimer",
            "Copy item" => "Copier l’élément",
            "Move item" => "Déplacer l’élément",
            "Paste here" => "Coller ici",
            "Copied “{name}”. Choose a destination and paste." => {
                "« {name} » copié. Choisissez une destination puis collez."
            }
            "Selected “{name}” for moving. Choose a destination and paste." => {
                "« {name} » sélectionné pour être déplacé. Choisissez une destination puis collez."
            }
            "Language" => "Langue",
            "System" => "Système",
            "System language" => "Langue du système",
            "Light" => "Clair",
            "Dark" => "Sombre",
            "Dark theme" => "Thème sombre",
            "Save" => "Enregistrer",
            "CONFIGURATION" => "Configuration",
            "Manage where configuration is stored, check for updates, and connect with GitHub." => {
                "Gérez l’emplacement du fichier de configuration, vérifiez les mises à jour et connectez GitHub."
            }
            "Version" => "Version",
            "Latest version" => "Dernière version",
            "Open the project page or review the latest release on GitHub." => {
                "Ouvrez la page du projet ou consultez la dernière version sur GitHub."
            }
            "Configuration folder" => "Dossier de configuration",
            "Settings file" => "Fichier de configuration",
            "R2 List saves its settings file in this folder." => {
                "R2 List enregistre son fichier de configuration dans ce dossier."
            }
            "Show hidden and system files" => "Afficher les fichiers cachés et système",
            "Use default" => "Utiliser par défaut",
            "Not checked" => "Non vérifié",
            "Checking…" => "Vérification…",
            "Checking GitHub for updates…" => "Vérification des mises à jour GitHub…",
            "Connected" => "Connecté",
            "Connection failed" => "Échec de la connexion",
            "Update available" => "Mise à jour disponible",
            "Up to date" => "À jour",
            "Update check failed" => "Échec de la vérification des mises à jour",
            "Choose folder" => "Choisir un dossier",
            "Reset" => "Réinitialiser",
            "ABOUT & UPDATES" => "À PROPOS ET MISES À JOUR",
            "Check for updates" => "Rechercher des mises à jour",
            "Test GitHub connection" => "Tester la connexion GitHub",
            "GitHub project" => "Projet GitHub",
            "Latest release" => "Dernière version",
            "Location settings" => "Paramètres de l’emplacement",
            "Location name" => "Nom de l’emplacement",
            "Public base URL" => "URL publique de base",
            "Object prefix (optional)" => "Préfixe de l’objet (facultatif)",
            "Bucket (optional)" => "Bucket (facultatif)",
            "Template: {base}/{key}" => "Modèle : {base}/{key}",
            "Template placeholders: {base}, {key}, {path}, {bucket}" => {
                "Espaces réservés du modèle : {base}, {key}, {path}, {bucket}"
            }
            "Select a file instead of a folder" => "Sélectionnez un fichier plutôt qu’un dossier",
            "Select a file first" => "Sélectionnez d’abord un fichier",
            "Select a file to preview" => "Sélectionnez un fichier à prévisualiser",
            "Use Open externally for this file type" => {
                "Utilisez Ouvrir pour ce type de fichier"
            }
            "Save location" => "Enregistrer l’emplacement",
            "Connect WebDAV" => "Connecter WebDAV",
            "Create folder" => "Créer le dossier",
            "Cancel" => "Annuler",
            "Close" => "Fermer",
            "Folder name" => "Nom du dossier",
            "Server address" => "Adresse du serveur",
            "Username" => "Nom d’utilisateur",
            "Password" => "Mot de passe",
            "Connect" => "Connecter",
            "Directory" => "Dossier",
            "Copy other formats" => "Copier d’autres formats",
            "Local storage" => "Stockage local",
            _ => english,
        },
        _ => english,
    };
    translated
}

fn apply_locale(window: &MainWindow, locale: &str) {
    let t = |english| translate(locale, english);
    window.set_text_workspace(t("WORKSPACE").into());
    window.set_text_object_workspace(t("OBJECT WORKSPACE").into());
    window.set_text_files(t("Files").into());
    window.set_text_url_rules(t("URL rules").into());
    window.set_text_settings(t("Settings").into());
    window.set_text_locations(t("LOCATIONS").into());
    window.set_text_add_location(t("Add location").into());
    window.set_text_local_folder(t("Local folder").into());
    window.set_text_webdav_server(t("WebDAV server").into());
    window.set_text_url_only(t("URL mapping only").into());
    window.set_text_external_sync(t("Sync handled externally").into());
    window.set_text_no_sync(t("R2 sync is not performed by this app").into());
    window.set_text_local(t("LOCAL").into());
    window.set_text_personal_workspace(t("Personal workspace").into());
    window.set_text_local_directory(t("LOCAL DIRECTORY").into());
    window.set_text_url_mapped(t("URL MAPPED").into());
    window.set_text_add_files(t("Add files").into());
    window.set_text_search(t("Search files").into());
    window.set_text_search_placeholder(t("Search files and folders…").into());
    window.set_text_new_folder(t("New folder").into());
    window.set_text_name(t("NAME").into());
    window.set_text_size(t("SIZE").into());
    window.set_text_modified(t("MODIFIED").into());
    window.set_text_empty_folder(t("This folder is empty").into());
    window.set_text_no_siblings(t("No other folders").into());
    window.set_text_empty_hint(t("Add files or create a folder to get started.").into());
    window.set_text_no_matches(t("No matching files").into());
    window.set_text_try_search(t("Try another search term").into());
    window.set_text_back_parent(t("Back to parent").into());
    window.set_text_edit_rules(t("Edit rules").into());
    window.set_text_edit_location(t("Edit location").into());
    window.set_text_more_actions(t("More actions").into());
    window.set_text_added(t("Added").into());
    window.set_text_storage_location(t("Storage location").into());
    window.set_text_public_status(t("Public status").into());
    window.set_text_configured(t("Configured").into());
    window.set_text_not_configured(t("Not configured").into());
    window.set_text_quick_look(t("QUICK LOOK").into());
    window.set_text_preview(t("Preview").into());
    window.set_text_choose_preview(t("Choose a file to preview").into());
    window.set_text_object_url(t("R2 OBJECT URL").into());
    window.set_text_copy_url(t("Copy URL").into());
    window.set_text_markdown(t("Markdown").into());
    window.set_text_html(t("HTML").into());
    window.set_text_open(t("Open").into());
    window.set_text_delete(t("Delete").into());
    window.set_text_copy_item(t("Copy item").into());
    window.set_text_move_item(t("Move item").into());
    window.set_text_paste_here(t("Paste here").into());
    window.set_text_language(t("Language").into());
    window.set_text_language_system(t("System language").into());
    window.set_text_theme_system(t("System").into());
    window.set_text_theme_light(t("Light").into());
    window.set_text_theme_dark(t("Dark").into());
    window.set_text_save(t("Save").into());
    window.set_text_settings_description(
        t("Manage where configuration is stored, check for updates, and connect with GitHub.").into(),
    );
    window.set_text_configuration(t("CONFIGURATION").into());
    window.set_text_version(t("Version").into());
    window.set_text_latest_version(t("Latest version").into());
    window.set_text_config_folder(t("Configuration folder").into());
    window.set_text_config_file(t("Settings file").into());
    window.set_text_config_hint(
        t("R2 List saves its settings file in this folder.").into(),
    );
    window.set_text_show_hidden(t("Show hidden and system files").into());
    window.set_text_choose_folder(t("Choose folder").into());
    window.set_text_reset(t("Use default").into());
    window.set_text_about_updates(t("ABOUT & UPDATES").into());
    window.set_text_check_updates(t("Check for updates").into());
    window.set_text_check_connection(t("Test GitHub connection").into());
    window.set_text_github_project(t("GitHub project").into());
    window.set_text_github_description(
        t("Open the project page or review the latest release on GitHub.").into(),
    );
    window.set_text_latest_release(t("Latest release").into());
    window.set_text_location_settings(t("Location settings").into());
    window.set_text_location_name(t("Location name").into());
    window.set_text_public_base_url(t("Public base URL").into());
    window.set_text_object_prefix(t("Object prefix (optional)").into());
    window.set_text_bucket(t("Bucket (optional)").into());
    window.set_text_template(t("Template: {base}/{key}").into());
    window.set_text_template_help(
        t("Template placeholders: {base}, {key}, {path}, {bucket}").into(),
    );
    window.set_text_save_location(t("Save location").into());
    window.set_text_connect_webdav(t("Connect WebDAV").into());
    window.set_text_create_folder(t("Create folder").into());
    window.set_text_cancel(t("Cancel").into());
    window.set_text_close(t("Close").into());
    window.set_text_folder_name(t("Folder name").into());
    window.set_text_server_address(t("Server address").into());
    window.set_text_username(t("Username").into());
    window.set_text_password(t("Password").into());
    window.set_text_connect(t("Connect").into());
    window.set_text_directory(t("Directory").into());
    window.set_text_copy_other(t("Copy other formats").into());
    window.set_text_local_storage(t("Local storage").into());
    if window.get_update_status_kind() == "idle" {
        window.set_update_status(t("Not checked").into());
    }
    if window.get_github_status_kind() == "idle" {
        window.set_github_status(t("Not checked").into());
    }
}

fn locale_code(name: &str) -> Result<&'static str, String> {
    match name {
        "English" => Ok("en"),
        "Français" => Ok("fr"),
        "简体中文" => Ok("zh-CN"),
        "繁體中文" => Ok("zh-TW"),
        "日本語" => Ok("ja"),
        "한국어" => Ok("ko"),
        _ => Err("Choose a supported language".into()),
    }
}

fn theme_name(theme: &str, locale: &str) -> &'static str {
    let english = match theme {
        "light" => "Light",
        "dark" => "Dark",
        _ => "System",
    };
    translate(locale, english)
}

fn theme_code(name: &str, locale: &str) -> Result<&'static str, String> {
    for candidate_locale in ["en", locale, "fr", "zh-CN", "zh-TW", "ja", "ko"] {
        for (code, english) in [("system", "System"), ("light", "Light"), ("dark", "Dark")] {
            if name == translate(candidate_locale, english) {
                return Ok(code);
            }
        }
    }
    Err("Choose a supported appearance".into())
}

fn compare_versions(current: &str, latest: &str) -> Result<Ordering, String> {
    let parse = |value: &str| -> Result<Vec<u64>, String> {
        let value = value.trim().trim_start_matches(['v', 'V']);
        if value.is_empty() {
            return Err("GitHub returned an invalid version".into());
        }
        value
            .split('.')
            .map(|part| {
                let numeric = part
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>();
                if numeric.is_empty() {
                    return Err("GitHub returned an invalid version".into());
                }
                numeric.parse::<u64>().map_err(|error| error.to_string())
            })
            .collect()
    };
    let mut left = parse(current)?;
    let mut right = parse(latest)?;
    let length = left.len().max(right.len());
    left.resize(length, 0);
    right.resize(length, 0);
    Ok(left.cmp(&right))
}

fn normalized_segments(value: &str) -> Result<Vec<String>, String> {
    let segments: Vec<_> = value
        .replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .map(str::to_owned)
        .collect();
    if segments.iter().any(|segment| segment == "..") {
        return Err("Paths cannot contain ..".into());
    }
    Ok(segments)
}

fn encode_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn build_object_url(relative_path: &str, rules: &UrlRulesConfig) -> Result<String, String> {
    let base = rules.base_url.trim().trim_end_matches('/');
    let parsed = url::Url::parse(base).map_err(|_| "Enter a complete HTTP or HTTPS base URL")?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("The base URL must be an HTTP(S) URL without a query or fragment".into());
    }
    let mut segments = normalized_segments(&rules.prefix)?;
    segments.extend(normalized_segments(relative_path)?);
    if segments.is_empty() {
        return Err("The object path cannot be empty".into());
    }
    let key = segments
        .iter()
        .map(|segment| encode_segment(segment))
        .collect::<Vec<_>>()
        .join("/");
    let Some(template) = rules
        .template
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(format!("{base}/{key}"));
    };
    if !template.contains("{key}") && !template.contains("{path}") {
        return Err("The template must contain {key} or {path}".into());
    }
    let bucket = normalized_segments(rules.bucket.as_deref().unwrap_or_default())?
        .iter()
        .map(|segment| encode_segment(segment))
        .collect::<Vec<_>>()
        .join("/");
    let template = template
        .replace("{base}", base)
        .replace("{bucket}", &bucket)
        .replace("{key}", &key)
        .replace("{path}", &key);
    if template.contains('{') || template.contains('}') {
        return Err("The template contains an unknown placeholder".into());
    }
    Ok(template)
}

fn format_markdown(name: &str, url: &str) -> String {
    let name = name
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    format!("[{name}]({url})")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn current_selection_absolute(state: &AppState) -> Result<PathBuf, String> {
    let selected = state
        .selected
        .as_deref()
        .ok_or_else(|| "Select a file first".to_string())?;
    let root = state
        .root
        .as_ref()
        .ok_or_else(|| "Choose a folder first".to_string())?;
    resolve_path(root, selected)
}

fn confirm_delete(path: &Path) -> bool {
    MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title("Delete item")
        .set_description(format!(
            "Permanently delete {}?",
            path.file_name().unwrap_or_default().to_string_lossy()
        ))
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == MessageDialogResult::Yes
}

pub fn run() {
    let window = MainWindow::new().expect("Could not create the Slint window");
    let (config, config_load_failed) = match load_user_config() {
        Ok(config) => (config, false),
        Err(error) => {
            window.set_status(format!("Could not load settings: {error}").into());
            (UserConfig::default(), true)
        }
    };
    let state = std::rc::Rc::new(std::cell::RefCell::new(AppState {
        config,
        config_load_failed,
        ..AppState::default()
    }));

    {
        let state = state.borrow();
        let locale = state
            .config
            .locale
            .clone()
            .unwrap_or_else(|| system_locale().to_owned());
        window.set_locale_follows_system(state.config.locale.is_none());
        window.set_locale(locale_name(&locale).into());
        window.set_locale_en("English".into());
        window.set_locale_fr("Français".into());
        window.set_locale_zh_cn("简体中文".into());
        window.set_locale_zh_tw("繁體中文".into());
        window.set_locale_ja("日本語".into());
        window.set_locale_ko("한국어".into());
        apply_locale(&window, &locale);
        let theme = state.config.theme.as_deref().unwrap_or("system");
        window.set_app_version(env!("CARGO_PKG_VERSION").into());
        window.set_system_dark(system_uses_dark_theme());
        window.set_theme_preference(theme_name(theme, &locale).into());
        window.set_theme_mode(theme.into());
        window.set_show_hidden(state.config.show_hidden);
        window.set_preferences_ready(true);
        match super::config_location() {
            Ok(location) => {
                window.set_config_directory(location.directory.into());
                window.set_config_file(location.config_path.into());
            }
            Err(error) => window.set_status(format!("Could not locate settings: {error}").into()),
        }
        update_window(&window, &state);
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_activate_location(move |path| {
            let result = state.borrow_mut().activate_location(path.as_str());
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        state.borrow_mut().editing_location = None;
                        window.set_search(SharedString::default());
                        update_location_settings(&window, &state.borrow());
                        window.set_preview_text(SharedString::default());
                        window.set_preview_image_visible(false);
                        window.set_status("Location loaded".into());
                        update_window(&window, &state.borrow());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_remove_location(move |path| {
            let result = state.borrow_mut().remove_location(path.as_str());
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        window.set_status("Location removed from the list".into());
                    }
                    Err(error) => show_error(&window, error),
                }
                if !state
                    .borrow()
                    .config
                    .directories
                    .iter()
                    .any(|directory| directory.path == path.as_str())
                {
                    window.set_location_editor_visible(false);
                }
                update_location_settings(&window, &state.borrow());
                update_window(&window, &state.borrow());
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_edit_location(move |path| {
            state.borrow_mut().editing_location = Some(path.to_string());
            if let Some(window) = weak.upgrade() {
                update_location_settings(&window, &state.borrow());
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_filter_changed(move || {
            if let Some(window) = weak.upgrade() {
                update_window(&window, &state.borrow());
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_view_changed(move |_| {
            if let Some(window) = weak.upgrade() {
                update_window(&window, &state.borrow());
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_sort_changed(move |_| {
            if let Some(window) = weak.upgrade() {
                update_window(&window, &state.borrow());
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_load_siblings(move || {
            if let Some(window) = weak.upgrade() {
                let siblings = sibling_folders(&state.borrow());
                window.set_siblings(ModelRc::new(VecModel::from(siblings)));
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_open_folder(move |path| {
            if path.trim().is_empty() {
                return;
            }
            let result = state.borrow_mut().open_directory(path.as_str());
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        state.borrow_mut().editing_location = None;
                        window.set_search(SharedString::default());
                        update_location_settings(&window, &state.borrow());
                        window.set_preview_text(SharedString::default());
                        window.set_preview_image_visible(false);
                        window.set_status("Folder opened".into());
                        update_window(&window, &state.borrow());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_connect_webdav(move |endpoint, username, password| {
            let result = state.borrow_mut().connect_remote(
                endpoint.as_str(),
                username.as_str(),
                password.as_str(),
            );
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        state.borrow_mut().editing_location = None;
                        window.set_webdav_visible(false);
                        window.set_search(SharedString::default());
                        update_location_settings(&window, &state.borrow());
                        window.set_preview_text(SharedString::default());
                        window.set_preview_image_visible(false);
                        window.set_status("WebDAV connected".into());
                        update_window(&window, &state.borrow());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_choose_folder(move || {
            if let Some(path) = FileDialog::new().pick_folder() {
                if let Some(window) = weak.upgrade() {
                    let result = state.borrow_mut().open_directory(&path.to_string_lossy());
                    match result {
                        Ok(()) => {
                            state.borrow_mut().editing_location = None;
                            window.set_search(SharedString::default());
                            update_location_settings(&window, &state.borrow());
                            window.set_status("Folder opened".into());
                            update_window(&window, &state.borrow());
                        }
                        Err(error) => show_error(&window, error),
                    }
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_navigate_up(move || {
            let result = {
                let mut state = state.borrow_mut();
                if state.current.is_empty() {
                    Ok(())
                } else {
                    let parent = Path::new(&state.current)
                        .parent()
                        .unwrap_or_else(|| Path::new(""))
                        .to_string_lossy()
                        .replace('\\', "/");
                    state.navigate(&parent)
                }
            };
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        window.set_search(SharedString::default());
                        update_window(&window, &state.borrow());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_refresh(move || {
            let result = state.borrow_mut().refresh();
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => update_window(&window, &state.borrow()),
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_create_folder(move |name| {
            let result = state.borrow_mut().create_folder(name.as_str());
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        window.set_new_folder_visible(false);
                        window.set_new_folder_name(SharedString::default());
                        window.set_status("Folder created".into());
                        update_window(&window, &state.borrow());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_navigate(move |path| {
            let result = state.borrow_mut().navigate(path.as_str());
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        window.set_search(SharedString::default());
                        window.set_preview_text(SharedString::default());
                        window.set_preview_image_visible(false);
                        update_window(&window, &state.borrow());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_import_files(move || {
            let Some(paths) = FileDialog::new().pick_files() else {
                return;
            };
            let result = state.borrow_mut().import_files(paths);
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        window.set_status("Files imported".into());
                        update_window(&window, &state.borrow());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_select_entry(move |path| {
            let mut state = state.borrow_mut();
            state.selected = Some(path.to_string());
            if let Some(window) = weak.upgrade() {
                update_window(&window, &state);
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_copy_selected(move |format| {
            let result = (|| {
                let state = state.borrow();
                let entry = state
                    .selected_entry()
                    .cloned()
                    .ok_or_else(|| "Select a file or folder first".to_string())?;
                let entries = state.files_under(&entry)?;
                if entries.is_empty() {
                    return Err("This folder contains no files".to_string());
                }
                let rules = state.rules();
                let links = entries
                    .iter()
                    .map(|entry| Ok((entry.name.clone(), build_object_url(&entry.path, &rules)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                let plain = links
                    .iter()
                    .map(|(_, url)| url.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let text = match format.as_str() {
                    "markdown" => links
                        .iter()
                        .map(|(name, url)| format_markdown(name, url))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    "html" => links
                        .iter()
                        .map(|(name, url)| {
                            format!("<a href=\"{}\">{}</a>", escape_html(url), escape_html(name))
                        })
                        .collect::<Vec<_>>()
                        .join("<br>\n"),
                    _ => plain.clone(),
                };
                let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
                if format == "html" {
                    clipboard
                        .set_html(text, Some(plain))
                        .map_err(|error| error.to_string())
                } else {
                    clipboard.set_text(text).map_err(|error| error.to_string())
                }
            })();
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => window.set_status("Copied to clipboard".into()),
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_preview_selected(move || {
            let locale = {
                let state = state.borrow();
                state
                    .config
                    .locale
                    .clone()
                    .unwrap_or_else(|| system_locale().to_owned())
            };
            let result = state.borrow().preview_content();
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(PreviewResult::Text(contents)) => {
                        window.set_preview_image_visible(false);
                        window.set_preview_text(contents.into());
                        window.set_status("Preview loaded".into());
                    }
                    Ok(PreviewResult::Image(image)) => {
                        window.set_preview_text(SharedString::default());
                        window.set_preview_image(image);
                        window.set_preview_image_visible(true);
                        window.set_status("Image preview loaded".into());
                    }
                    Ok(PreviewResult::External) => {
                        window.set_preview_text(
                            translate(&locale, "Opened with the system default application.").into(),
                        );
                        window.set_preview_image_visible(false);
                        let result = state.borrow_mut().open_selected();
                        match result {
                            Ok(()) => window.set_status("Opened externally".into()),
                            Err(error) => show_error(&window, error),
                        }
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_open_selected(move || {
            let result = state.borrow_mut().open_selected();
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => window.set_status("Opened externally".into()),
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_transfer_selected(move |mode| {
            let result = state.borrow_mut().start_transfer(mode.as_str());
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => window.set_status(format!("Ready to {mode} selected item").into()),
                    Err(error) => show_error(&window, error),
                }
                update_window(&window, &state.borrow());
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_cancel_transfer(move || {
            state.borrow_mut().cancel_transfer();
            if let Some(window) = weak.upgrade() {
                update_window(&window, &state.borrow());
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_paste_here(move || {
            let result = state.borrow_mut().paste_transfer();
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        window.set_transfer_pending(false);
                        window.set_status("Item transferred".into());
                        update_window(&window, &state.borrow());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_paste_here_at(move |destination| {
            let result = state.borrow_mut().paste_transfer_to(destination.as_str());
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        window.set_transfer_pending(false);
                        window.set_status("Item transferred".into());
                        update_window(&window, &state.borrow());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_delete_selected(move || {
            let is_remote = state.borrow().remote.is_some();
            let target = if is_remote {
                state
                    .borrow()
                    .selected
                    .clone()
                    .map(PathBuf::from)
                    .ok_or_else(|| "Select an item first".to_string())
            } else {
                current_selection_absolute(&state.borrow())
            };
            let target = match target {
                Ok(target) => target,
                Err(error) => {
                    if let Some(window) = weak.upgrade() {
                        show_error(&window, error);
                    }
                    return;
                }
            };
            let should_delete = if is_remote {
                MessageDialog::new()
                    .set_level(MessageLevel::Warning)
                    .set_title("Delete item")
                    .set_description(format!("Permanently delete {}?", target.display()))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show()
                    == MessageDialogResult::Yes
            } else {
                confirm_delete(&target)
            };
            if !should_delete {
                return;
            }
            let result = state.borrow_mut().delete_selected();
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        window.set_status("Item deleted".into());
                        window.set_preview_text(SharedString::default());
                        window.set_preview_image_visible(false);
                        update_window(&window, &state.borrow());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_preview_rules(move |base_url, prefix, bucket, template| {
            let rules = UrlRulesConfig {
                base_url: base_url.to_string(),
                prefix: prefix.to_string(),
                bucket: (!bucket.trim().is_empty()).then(|| bucket.to_string()),
                template: (!template.trim().is_empty()).then(|| template.to_string()),
            };
            let preview = object_url_preview(&state.borrow(), &rules);
            if let Some(window) = weak.upgrade() {
                window.set_location_preview(preview.into());
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_save_settings(
            move |label, base_url, prefix, bucket, template, endpoint, username, password| {
                let path = {
                    let state = state.borrow();
                    state
                        .editing_location
                        .clone()
                        .unwrap_or_else(|| state.active_location())
                };
                let result = state.borrow_mut().set_rules(
                    &path,
                    label.to_string(),
                    base_url.to_string(),
                    prefix.to_string(),
                    bucket.to_string(),
                    template.to_string(),
                    endpoint.to_string(),
                    username.to_string(),
                    password.to_string(),
                );
                if let Some(window) = weak.upgrade() {
                    match result {
                        Ok(path) => {
                            let reconnect = state
                                .borrow()
                                .config
                                .directories
                                .iter()
                                .find(|directory| directory.path == path)
                                .is_some_and(|directory| {
                                    directory.kind == "webdav"
                                        && state.borrow().config.active_directory.as_deref()
                                            == Some(path.as_str())
                                });
                            if reconnect {
                                if let Err(error) = state.borrow_mut().activate_location(&path) {
                                    let mut state = state.borrow_mut();
                                    state.remote = None;
                                    state.entries.clear();
                                    state.current.clear();
                                    state.selected = None;
                                    show_error(&window, error);
                                    update_location_settings(&window, &state);
                                    update_window(&window, &state);
                                    return;
                                }
                            }
                            window.set_location_editor_visible(false);
                            window.set_status("Location settings saved".into());
                            update_location_settings(&window, &state.borrow());
                            update_window(&window, &state.borrow());
                        }
                        Err(error) => show_error(&window, error),
                    }
                }
            },
        );
    }

    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_check_updates(move || {
            if let Some(window) = weak.upgrade() {
                let locale = state
                    .borrow()
                    .config
                    .locale
                    .clone()
                    .unwrap_or_else(|| system_locale().to_owned());
                let t = |english| translate(&locale, english);
                window.set_update_status_kind("checking".into());
                window.set_update_status(t("Checking GitHub for updates…").into());
                match AppState::default().check_updates() {
                    Ok(status) => {
                        let (kind, text) = localize_update_result(&locale, &status);
                        window.set_update_status_kind(kind.into());
                        window.set_update_status(text.into());
                    }
                    Err(error) => {
                        window.set_update_status_kind("error".into());
                        window.set_update_status(
                            format!("{}: {error}", t("Update check failed")).into(),
                        );
                    }
                }
            }
        });
    }

    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_check_github(move || {
            if let Some(window) = weak.upgrade() {
                let locale = state
                    .borrow()
                    .config
                    .locale
                    .clone()
                    .unwrap_or_else(|| system_locale().to_owned());
                let t = |english| translate(&locale, english);
                window.set_github_status_kind("checking".into());
                window.set_github_status(t("Checking…").into());
                match AppState::default().check_github_connection() {
                    Ok(()) => {
                        window.set_github_status_kind("ok".into());
                        window.set_github_status(t("Connected").into());
                    }
                    Err(error) => {
                        window.set_github_status_kind("error".into());
                        window.set_github_status(
                            format!("{}: {error}", t("Connection failed")).into(),
                        );
                    }
                }
            }
        });
    }

    {
        let weak = window.as_weak();
        window.on_open_project(move |address| {
            let result = url::Url::parse(address.as_str())
                .map_err(|error| error.to_string())
                .and_then(|url| {
                    if url.scheme() != "https" || url.host_str() != Some("github.com") {
                        return Err("Only the GitHub project links can be opened".into());
                    }
                    open::that(url.as_str()).map_err(|error| error.to_string())
                });
            if let (Err(error), Some(window)) = (result, weak.upgrade()) {
                show_error(&window, error);
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_save_preferences(move |locale, theme| {
            let current_locale = state
                .borrow()
                .config
                .locale
                .clone()
                .unwrap_or_else(|| system_locale().to_owned());
            let result = (if locale.is_empty() {
                Ok(None)
            } else {
                locale_code(locale.as_str()).map(|locale| Some(locale.to_string()))
            })
            .and_then(|locale| {
                let locale_for_ui = locale.as_deref().unwrap_or(&current_locale).to_owned();
                let theme = theme_code(theme.as_str(), &locale_for_ui)?;
                state
                    .borrow_mut()
                    .save_preferences(locale, theme)
                    .map(|()| (locale_for_ui, theme))
            });
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok((locale, theme)) => {
                        window.set_preferences_ready(false);
                        apply_locale(&window, &locale);
                        window.set_theme_preference(theme_name(theme, &locale).into());
                        if theme == "system" {
                            window.set_system_dark(system_uses_dark_theme());
                        }
                        window.set_theme_mode(theme.into());
                        {
                            let state_ref = state.borrow();
                            update_window(&window, &state_ref);
                            if state_ref.selected_entry().is_some() {
                                update_preview(&window, &state_ref, &locale);
                            }
                        }
                        window.set_preferences_ready(true);
                        window.set_status("Preferences saved".into());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_show_hidden_changed(move |value| {
            let mut state = state.borrow_mut();
            state.config.show_hidden = value;
            if let Err(error) = save_user_config(state.config.clone()) {
                if let Some(window) = weak.upgrade() {
                    show_error(&window, error);
                }
                return;
            }
            if let Some(window) = weak.upgrade() {
                update_window(&window, &state);
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_choose_config_directory(move || {
            let Some(path) = FileDialog::new().pick_folder() else {
                return;
            };
            let result = state
                .borrow_mut()
                .change_config_directory(path.to_string_lossy().into_owned());
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(location) => {
                        window.set_config_directory(location.directory.into());
                        window.set_config_file(location.config_path.into());
                        window.set_status("Settings folder changed".into());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_reset_config_directory(move || {
            let result = state.borrow_mut().change_config_directory(String::new());
            if let Some(window) = weak.upgrade() {
                match result {
                    Ok(location) => {
                        window.set_config_directory(location.directory.into());
                        window.set_config_file(location.config_path.into());
                        window.set_status("Settings folder reset".into());
                    }
                    Err(error) => show_error(&window, error),
                }
            }
        });
    }

    {
        let state = state.clone();
        let weak = window.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(30), move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            let active = if state.config_load_failed {
                None
            } else {
                state
                    .config
                    .active_directory
                    .clone()
                    .or_else(default_directory)
            };
            if let Some(path) = active {
                let saved = state
                    .config
                    .directories
                    .iter()
                    .find(|directory| directory.path == path)
                    .cloned();
                let result =
                    if let Some(directory) = saved.filter(|directory| directory.kind == "webdav") {
                        window.set_webdav_endpoint(directory.endpoint.clone().into());
                        window.set_webdav_username(directory.username.clone().into());
                        window.set_webdav_password(directory.password.clone().into());
                        state.connect_remote(
                            &directory.endpoint,
                            &directory.username,
                            &directory.password,
                        )
                    } else {
                        state.open_directory(&path)
                    };
                if let Err(error) = result {
                    state.root = None;
                    state.remote = None;
                    window.set_status(
                        format!("Could not restore the previous location: {error}").into(),
                    );
                } else {
                    window.set_status("Location loaded".into());
                }
            }
            update_location_settings(&window, &state);
            update_window(&window, &state);
        });
    }

    window.run().expect("Slint event loop failed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_url_encodes_each_path_segment() {
        let rules = UrlRulesConfig {
            base_url: "https://cdn.example.test".into(),
            prefix: "media set".into(),
            bucket: None,
            template: None,
        };
        assert_eq!(
            build_object_url("a file/100%.png", &rules).unwrap(),
            "https://cdn.example.test/media%20set/a%20file/100%25.png"
        );
    }

    #[test]
    fn object_url_rejects_parent_segments() {
        let rules = UrlRulesConfig::default();
        assert!(build_object_url("../private.png", &rules).is_err());
    }

    #[test]
    fn markdown_escapes_link_names() {
        assert_eq!(
            format_markdown("a [file].png", "https://cdn.example.test/a.png"),
            "[a \\[file\\].png](https://cdn.example.test/a.png)"
        );
    }

    #[test]
    fn html_escapes_attribute_and_text_content() {
        assert_eq!(
            format!(
                "<a href=\"{}\">{}</a>",
                escape_html("\"&"),
                escape_html("<file>")
            ),
            "<a href=\"&quot;&amp;\">&lt;file&gt;</a>"
        );
    }

    #[test]
    fn extracts_namespaced_office_text_and_unescapes_entities() {
        let xml = r#"<w:document xmlns:w="urn:word"><w:p><w:r><w:t>Rock &amp; Roll</w:t></w:r><w:r><w:t>!</w:t></w:r></w:p></w:document>"#;
        assert_eq!(
            xml_grouped_text(xml, "p", "t").unwrap(),
            vec!["Rock & Roll!".to_string()]
        );
    }

    #[test]
    fn normalizes_webdav_location_labels_without_dropping_base_paths() {
        assert_eq!(
            normalize_webdav_endpoint(" https://dav.example.com/dav/files/alice/ ").unwrap(),
            "https://dav.example.com/dav/files/alice"
        );
        assert_eq!(
            webdav_location_name("https://dav.example.com/dav/files/alice"),
            "dav.example.com"
        );
        let legacy = DirectoryConfig {
            path: "webdav://https://dav.example.com/dav".into(),
            kind: "webdav".into(),
            label: "https://dav.example.com/dav".into(),
            endpoint: "https://dav.example.com/dav".into(),
            username: String::new(),
            password: String::new(),
            rules: UrlRulesConfig::default(),
        };
        assert_eq!(location_display_name(&legacy), "dav.example.com");
    }

    #[test]
    fn maps_system_locales_to_supported_languages() {
        for (locale, expected) in [
            ("fr-CA", "fr"),
            ("zh-CN", "zh-CN"),
            ("zh-Hans-SG", "zh-CN"),
            ("zh-TW", "zh-TW"),
            ("zh-Hant-HK", "zh-TW"),
            ("ja-JP", "ja"),
            ("ko-KR", "ko"),
            ("en-GB", "en"),
            ("de-DE", "en"),
        ] {
            assert_eq!(supported_system_locale(locale), expected);
        }
    }

    #[test]
    fn maps_language_labels_to_persisted_locale_codes() {
        for (code, name) in [
            ("en", "English"),
            ("fr", "Français"),
            ("zh-CN", "简体中文"),
            ("zh-TW", "繁體中文"),
            ("ja", "日本語"),
            ("ko", "한국어"),
        ] {
            assert_eq!(locale_name(code), name);
            assert_eq!(locale_code(name).unwrap(), code);
        }
        assert_eq!(theme_code("深色", "zh-CN").unwrap(), "dark");
        assert_eq!(theme_code("Sombre", "fr").unwrap(), "dark");
        assert_eq!(theme_code("System", "en").unwrap(), "system");
        assert_eq!(translate("ja", "Copy URL"), "URL をコピー");
        for locale in ["fr", "zh-CN", "zh-TW", "ja", "ko"] {
            assert_ne!(translate(locale, "No matching files"), "No matching files");
            assert_ne!(
                translate(locale, "Try another search term"),
                "Try another search term"
            );
        }
    }

    #[test]
    fn compares_release_versions_without_lexicographic_ordering() {
        assert_eq!(
            compare_versions("0.0.9", "v0.0.10").unwrap(),
            Ordering::Less
        );
        assert_eq!(
            compare_versions("1.2.0", "1.1.99").unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("v2.3.4", "2.3.4").unwrap(),
            Ordering::Equal
        );
        assert!(compare_versions("1.x", "2.0.0").is_err());
    }

    #[test]
    fn formats_file_times_for_the_selected_locale() {
        let timestamp = 1_760_000_000_000;
        assert!(format_modified(timestamp, "zh-CN").contains('年'));
        assert!(format_modified(timestamp, "ja").contains('月'));
        assert!(format_modified(timestamp, "fr").contains('/'));
        assert!(format_modified(timestamp, "en").contains(','));
        assert!(format_modified(0, "zh-CN").is_empty());
    }

    #[test]
    fn markdown_preview_removes_common_formatting_but_keeps_link_labels() {
        assert_eq!(
            markdown_plain_text("# **Title**\n- [Guide](https://example.test)"),
            "Title\n• Guide"
        );
    }

    #[test]
    fn csv_preview_keeps_quoted_delimiters_and_embedded_newlines() {
        assert_eq!(
            tabular_text("name,note\r\nAlice,\"a,b\"\r\nBob,\"two\nlines\"", ','),
            "name  |  note\nAlice  |  a,b\nBob  |  two\nlines"
        );
    }

    #[test]
    fn failed_directory_navigation_keeps_the_current_location() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = AppState {
            root: Some(directory.path().to_path_buf()),
            current: String::new(),
            config_load_failed: true,
            ..AppState::default()
        };
        assert!(state.navigate("missing").is_err());
        assert!(state.current.is_empty());
        assert_eq!(state.root.as_deref(), Some(directory.path()));
    }

    #[test]
    fn opening_a_file_as_a_directory_does_not_replace_current_state() {
        let directory = tempfile::tempdir().unwrap();
        let file_path = directory.path().join("not-a-directory.txt");
        fs::write(&file_path, "content").unwrap();
        let mut state = AppState {
            root: Some(directory.path().to_path_buf()),
            current: "subfolder".into(),
            config_load_failed: true,
            ..AppState::default()
        };
        assert!(state.open_directory(file_path.to_str().unwrap()).is_err());
        assert_eq!(state.root.as_deref(), Some(directory.path()));
        assert_eq!(state.current, "subfolder");
    }

    #[test]
    fn webdav_credentials_survive_config_serialization() {
        let config = UserConfig {
            directories: vec![DirectoryConfig {
                path: "webdav://https://dav.example.com".into(),
                kind: "webdav".into(),
                label: "https://dav.example.com".into(),
                endpoint: "https://dav.example.com".into(),
                username: "alice".into(),
                password: "example-password".into(),
                rules: UrlRulesConfig::default(),
            }],
            ..UserConfig::default()
        };
        let serialized = serde_json::to_string(&config).unwrap();
        let restored: UserConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.directories[0].username, "alice");
        assert_eq!(restored.directories[0].password, "example-password");
    }
}
