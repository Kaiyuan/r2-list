use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::UNIX_EPOCH,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use remotefs::{
    RemoteFs,
    fs::{ReadOptions, WriteOptions},
};
use remotefs_smb::{SmbCredentials, SmbFs, SmbOptions};
use remotefs_webdav::{Auth, WebDAVFs};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

#[derive(Default)]
struct LocalDirectory(Mutex<Option<PathBuf>>);

#[derive(Default)]
struct RemoteDirectory(Mutex<Option<RemoteConnection>>);

struct RemoteConnection {
    id: String,
    client: Box<dyn RemoteFs>,
}

fn remote_runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    if let Some(runtime) = RUNTIME.get() {
        return Ok(runtime);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let _ = RUNTIME.set(runtime);
    RUNTIME
        .get()
        .ok_or_else(|| "Could not start network runtime".to_string())
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryConfig {
    path: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    endpoint: String,
    #[serde(default)]
    share: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    workgroup: String,
    #[serde(default)]
    rules: UrlRulesConfig,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UrlRulesConfig {
    base_url: String,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    bucket: Option<String>,
    #[serde(default)]
    template: Option<String>,
}

impl Default for UrlRulesConfig {
    fn default() -> Self {
        Self {
            base_url: "https://cdn.studio-example.com".to_string(),
            prefix: "northstar/media".to_string(),
            bucket: None,
            template: None,
        }
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserConfig {
    #[serde(default)]
    directories: Vec<DirectoryConfig>,
    #[serde(default)]
    active_directory: Option<String>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    theme: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryEntry {
    path: String,
    name: String,
    kind: String,
    size_bytes: u64,
    modified_ms: u64,
    extension: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryListing {
    root_path: String,
    current_path: String,
    entries: Vec<DirectoryEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewData {
    mime_type: String,
    data_base64: String,
    size_bytes: u64,
}

fn file_kind(extension: &str) -> &'static str {
    match extension {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif" | "ico" => "image",
        "mp4" | "m4v" | "mov" | "webm" | "ogv" | "mkv" => "video",
        "mp3" | "wav" | "ogg" | "oga" | "m4a" | "aac" | "flac" | "opus" => "audio",
        "pdf" | "doc" | "docx" | "odt" | "rtf" | "xls" | "xlsx" | "ods" | "ppt" | "pptx"
        | "odp" => "document",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => "archive",
        "txt" | "md" | "markdown" | "csv" | "tsv" | "json" | "jsonl" | "xml" | "html" | "htm"
        | "css" | "js" | "jsx" | "ts" | "tsx" | "rs" | "py" | "toml" | "yaml" | "yml" | "ini"
        | "log" | "sql" | "sh" | "bat" | "ps1" | "c" | "h" | "cpp" | "java" => "text",
        _ => "file",
    }
}

fn mime_type(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "opus" => "audio/opus",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "csv" => "text/csv",
        "json" => "application/json",
        _ => "text/plain",
    }
}

fn active_root(state: &LocalDirectory) -> Result<PathBuf, String> {
    state
        .0
        .lock()
        .map_err(|_| "Directory state unavailable".to_string())?
        .clone()
        .ok_or_else(|| "Choose a local directory first".to_string())
}

fn display_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(unc_path) = value.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc_path}");
        }
        if let Some(normal_path) = value.strip_prefix(r"\\?\") {
            return normal_path.to_string();
        }
    }
    value.into_owned()
}

fn remote_absolute_path(relative: &str) -> Result<PathBuf, String> {
    let mut path = PathBuf::from("/");
    for component in relative.split('/') {
        if component.is_empty() {
            continue;
        }
        if component == "." || component == ".." || component.contains('\\') {
            return Err("Invalid remote path".to_string());
        }
        path.push(component);
    }
    Ok(path)
}

fn join_remote_path(parent: &str, name: &str) -> String {
    let parent = parent.trim_matches('/');
    let name = name.trim_matches('/');
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn normalize_smb_endpoint(endpoint: &str, share: &str) -> Result<(String, String), String> {
    let endpoint = endpoint.trim();
    let address = if endpoint
        .get(..6)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("smb://"))
    {
        &endpoint[6..]
    } else {
        endpoint
    };
    let address = address.replace('\\', "/");
    let mut parts = address
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty());
    let host = parts.next().unwrap_or_default().trim();
    if host.is_empty() {
        return Err("Enter a valid SMB server address".to_string());
    }
    let path_share = parts.next().unwrap_or_default().trim();
    let explicit_share = share.trim().trim_matches(['/', '\\']);
    let share = if explicit_share.is_empty() {
        path_share
    } else {
        explicit_share
    };
    if share.is_empty() || share.contains(['/', '\\']) {
        return Err("Enter a valid SMB share name".to_string());
    }
    Ok((host.to_string(), share.to_string()))
}

fn remote_listing(
    client: &dyn RemoteFs,
    root_id: &str,
    current_path: &str,
) -> Result<DirectoryListing, String> {
    let current = remote_absolute_path(current_path)?;
    let mut entries = client
        .list_dir(&current)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|entry| {
            let name = entry.name();
            let relative_path = join_remote_path(current_path, &name);
            let extension = entry.extension().unwrap_or_default().to_ascii_lowercase();
            let modified_ms = entry
                .metadata()
                .modified
                .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                .map(|value| value.as_millis().min(u64::MAX as u128) as u64)
                .unwrap_or_default();
            DirectoryEntry {
                path: relative_path,
                name,
                kind: if entry.is_dir() {
                    "directory"
                } else {
                    file_kind(&extension)
                }
                .to_string(),
                size_bytes: entry.metadata().size.unwrap_or_default(),
                modified_ms,
                extension,
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .kind
            .cmp(&left.kind)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(DirectoryListing {
        root_path: root_id.to_string(),
        current_path: current_path.to_string(),
        entries,
    })
}

fn resolve_path(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative_path);
    if relative.is_absolute() {
        return Err("Absolute paths are not allowed".to_string());
    }
    let path = fs::canonicalize(root.join(relative)).map_err(|error| error.to_string())?;
    if !path.starts_with(root) {
        return Err("Path is outside the selected directory".to_string());
    }
    Ok(path)
}

fn list_directory(root: &Path, current: &Path) -> Result<Vec<DirectoryEntry>, String> {
    let mut entries = Vec::new();
    for item in fs::read_dir(current).map_err(|error| error.to_string())? {
        let item = item.map_err(|error| error.to_string())?;
        let path = match fs::canonicalize(item.path()) {
            Ok(path) if path.starts_with(root) => path,
            _ => continue,
        };
        let metadata = item.metadata().map_err(|error| error.to_string())?;
        let relative_path = path.strip_prefix(root).map_err(|error| error.to_string())?;
        let name = item.file_name().to_string_lossy().into_owned();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_millis().min(u64::MAX as u128) as u64)
            .unwrap_or_default();
        entries.push(DirectoryEntry {
            path: relative_path.to_string_lossy().replace('\\', "/"),
            name,
            kind: if metadata.is_dir() {
                "directory"
            } else {
                file_kind(&extension)
            }
            .to_string(),
            size_bytes: if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
            modified_ms,
            extension,
        });
    }
    entries.sort_by(|left, right| {
        right
            .kind
            .cmp(&left.kind)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

#[tauri::command]
fn default_directory() -> Option<String> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let home = PathBuf::from(home);
    let documents = home.join("Documents");
    Some(
        if documents.is_dir() { documents } else { home }
            .to_string_lossy()
            .into_owned(),
    )
}

fn user_config_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .ok_or_else(|| "Could not locate the user home directory".to_string())?;
    Ok(PathBuf::from(home).join("r2-list.comfig"))
}

#[tauri::command]
fn load_user_config() -> Result<UserConfig, String> {
    let path = user_config_path()?;
    if !path.exists() {
        return Ok(UserConfig::default());
    }
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&contents).map_err(|error| error.to_string())
}

#[tauri::command]
fn save_user_config(config: UserConfig) -> Result<(), String> {
    let path = user_config_path()?;
    let contents = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
    fs::write(path, contents).map_err(|error| error.to_string())
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
fn clear_directory(
    state: State<'_, LocalDirectory>,
    remote: State<'_, RemoteDirectory>,
) -> Result<(), String> {
    *state
        .0
        .lock()
        .map_err(|_| "Directory state unavailable".to_string())? = None;
    remote
        .0
        .lock()
        .map_err(|_| "Remote directory state unavailable".to_string())?
        .take();
    Ok(())
}

#[tauri::command]
fn open_directory(
    path: String,
    app: AppHandle,
    state: State<'_, LocalDirectory>,
    remote: State<'_, RemoteDirectory>,
) -> Result<DirectoryListing, String> {
    let root = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !root.is_dir() {
        return Err("The selected path is not a directory".to_string());
    }
    app.asset_protocol_scope()
        .allow_directory(&root, true)
        .map_err(|error| error.to_string())?;
    *state
        .0
        .lock()
        .map_err(|_| "Directory state unavailable".to_string())? = Some(root.clone());
    remote
        .0
        .lock()
        .map_err(|_| "Remote directory state unavailable".to_string())?
        .take();
    let entries = list_directory(&root, &root)?;
    Ok(DirectoryListing {
        root_path: display_path(&root),
        current_path: String::new(),
        entries,
    })
}

#[tauri::command]
fn connect_remote_directory(
    protocol: String,
    endpoint: String,
    share: String,
    username: String,
    password: String,
    workgroup: String,
    state: State<'_, RemoteDirectory>,
    local: State<'_, LocalDirectory>,
) -> Result<DirectoryListing, String> {
    let endpoint = endpoint.trim().trim_end_matches('/').to_string();
    if endpoint.is_empty() {
        return Err("Enter a server address".to_string());
    }
    let runtime = remote_runtime()?;
    let mut client: Box<dyn RemoteFs> = match protocol.as_str() {
        "smb" => {
            let (server, share) = normalize_smb_endpoint(&endpoint, &share)?;
            let credentials = SmbCredentials::default()
                .server(&server)
                .share(&share)
                .username(&username)
                .password(&password);
            let credentials = if workgroup.trim().is_empty() {
                credentials
            } else {
                credentials.workgroup(workgroup.trim())
            };
            Box::new(
                SmbFs::try_new(credentials, SmbOptions::default())
                    .map_err(|error| error.to_string())?
                    .into_blocking(runtime.handle().clone()),
            )
        }
        "webdav" => {
            let url = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
                endpoint.clone()
            } else {
                format!("https://{endpoint}")
            };
            Box::new(
                WebDAVFs::new(&url, Auth::basic(&username, &password))
                    .map_err(|error| error.to_string())?
                    .into_blocking(runtime.handle().clone()),
            )
        }
        _ => return Err("Unsupported network protocol".to_string()),
    };
    client.connect().map_err(|error| error.to_string())?;
    let id = if protocol == "smb" {
        let (server, share) = normalize_smb_endpoint(&endpoint, &share)?;
        format!("smb://{server}/{share}")
    } else {
        format!("webdav://{endpoint}")
    };
    let listing = remote_listing(client.as_ref(), &id, "")?;
    *state
        .0
        .lock()
        .map_err(|_| "Remote directory state unavailable".to_string())? =
        Some(RemoteConnection { id, client });
    *local
        .0
        .lock()
        .map_err(|_| "Directory state unavailable".to_string())? = None;
    Ok(listing)
}

#[tauri::command]
fn read_remote_directory(
    path: String,
    state: State<'_, RemoteDirectory>,
) -> Result<DirectoryListing, String> {
    let connection = state
        .0
        .lock()
        .map_err(|_| "Remote directory state unavailable".to_string())?;
    let connection = connection
        .as_ref()
        .ok_or_else(|| "Connect to a network location first".to_string())?;
    remote_listing(connection.client.as_ref(), &connection.id, &path)
}

#[tauri::command]
fn create_remote_directory(
    path: String,
    name: String,
    state: State<'_, RemoteDirectory>,
) -> Result<DirectoryListing, String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err("Enter a valid folder name".to_string());
    }
    let connection = state
        .0
        .lock()
        .map_err(|_| "Remote directory state unavailable".to_string())?;
    let connection = connection
        .as_ref()
        .ok_or_else(|| "Connect to a network location first".to_string())?;
    let target = remote_absolute_path(&path)?.join(name);
    connection
        .client
        .create_dir(&target, None)
        .map_err(|error| error.to_string())?;
    remote_listing(connection.client.as_ref(), &connection.id, &path)
}

#[tauri::command]
fn import_remote_files(
    paths: Vec<String>,
    directory: String,
    state: State<'_, RemoteDirectory>,
) -> Result<DirectoryListing, String> {
    let connection = state
        .0
        .lock()
        .map_err(|_| "Remote directory state unavailable".to_string())?;
    let connection = connection
        .as_ref()
        .ok_or_else(|| "Connect to a network location first".to_string())?;
    let target_directory = remote_absolute_path(&directory)?;
    for source in paths {
        let source = fs::canonicalize(source).map_err(|error| error.to_string())?;
        if !source.is_file() {
            return Err("Only files can be added to this directory".to_string());
        }
        let name = source
            .file_name()
            .ok_or_else(|| "The selected file has no name".to_string())?;
        let mut target = target_directory.join(name);
        let mut suffix = 2;
        while connection
            .client
            .exists(&target)
            .map_err(|error| error.to_string())?
        {
            let stem = source.file_stem().unwrap_or_default().to_string_lossy();
            let extension = source.extension().map(|value| value.to_string_lossy());
            let renamed = if let Some(extension) = extension {
                format!("{stem} ({suffix}).{extension}")
            } else {
                format!("{stem} ({suffix})")
            };
            target = target_directory.join(renamed);
            suffix += 1;
        }
        let bytes = fs::read(&source).map_err(|error| error.to_string())?;
        connection
            .client
            .write_file(
                &target,
                &WriteOptions::default().size_hint(bytes.len() as u64),
                &mut Cursor::new(bytes),
            )
            .map_err(|error| error.to_string())?;
    }
    remote_listing(connection.client.as_ref(), &connection.id, &directory)
}

#[tauri::command]
fn read_remote_preview(
    path: String,
    state: State<'_, RemoteDirectory>,
) -> Result<PreviewData, String> {
    const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;
    let connection = state
        .0
        .lock()
        .map_err(|_| "Remote directory state unavailable".to_string())?;
    let connection = connection
        .as_ref()
        .ok_or_else(|| "Connect to a network location first".to_string())?;
    let path = remote_absolute_path(&path)?;
    let metadata = connection
        .client
        .stat(&path)
        .map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("The selected path is not a file".to_string());
    }
    let extension = metadata
        .path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let preview_limit = match file_kind(&extension) {
        "text" => 2 * 1024 * 1024,
        "document" => 16 * 1024 * 1024,
        _ => MAX_PREVIEW_BYTES,
    };
    if metadata.metadata().size.unwrap_or_default() > preview_limit {
        return Err(format!(
            "This file exceeds the {} MB preview limit",
            preview_limit / 1024 / 1024
        ));
    }
    let mut bytes = Vec::new();
    connection
        .client
        .read_file(&path, &ReadOptions::default(), &mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(PreviewData {
        mime_type: mime_type(&extension).to_string(),
        size_bytes: bytes.len() as u64,
        data_base64: STANDARD.encode(bytes),
    })
}

#[tauri::command]
fn read_directory(
    path: String,
    state: State<'_, LocalDirectory>,
) -> Result<DirectoryListing, String> {
    let root = active_root(&state)?;
    let current = resolve_path(&root, &path)?;
    if !current.is_dir() {
        return Err("The selected path is not a directory".to_string());
    }
    let current_path = current
        .strip_prefix(&root)
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let entries = list_directory(&root, &current)?;
    Ok(DirectoryListing {
        root_path: display_path(&root),
        current_path,
        entries,
    })
}

#[tauri::command]
fn create_directory(
    path: String,
    name: String,
    state: State<'_, LocalDirectory>,
) -> Result<DirectoryListing, String> {
    let name = name.trim();
    let name_path = Path::new(name);
    if name.is_empty()
        || name == "."
        || name == ".."
        || name_path.components().count() != 1
        || !matches!(
            name_path.components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err("Enter a valid folder name".to_string());
    }
    let root = active_root(&state)?;
    let current = resolve_path(&root, &path)?;
    if !current.is_dir() {
        return Err("The selected path is not a directory".to_string());
    }
    fs::create_dir(current.join(name)).map_err(|error| error.to_string())?;
    let current_path = current
        .strip_prefix(&root)
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let entries = list_directory(&root, &current)?;
    Ok(DirectoryListing {
        root_path: display_path(&root),
        current_path,
        entries,
    })
}

fn available_destination(directory: &Path, name: &str) -> PathBuf {
    let source = Path::new(name);
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let extension = source.extension().map(|value| value.to_string_lossy());
    for suffix in 1.. {
        let candidate_name = if suffix == 1 {
            name.to_string()
        } else if let Some(extension) = &extension {
            format!("{stem} ({suffix}).{extension}")
        } else {
            format!("{stem} ({suffix})")
        };
        let candidate = directory.join(candidate_name);
        if fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    unreachable!()
}

#[tauri::command]
fn import_files(
    paths: Vec<String>,
    directory: String,
    state: State<'_, LocalDirectory>,
) -> Result<DirectoryListing, String> {
    let root = active_root(&state)?;
    let current = resolve_path(&root, &directory)?;
    if !current.is_dir() {
        return Err("The selected path is not a directory".to_string());
    }
    for source in paths {
        let source = fs::canonicalize(source).map_err(|error| error.to_string())?;
        let metadata = fs::metadata(&source).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("Only files can be added to this directory".to_string());
        }
        let name = source
            .file_name()
            .ok_or_else(|| "The selected file has no name".to_string())?
            .to_string_lossy();
        let destination = available_destination(&current, &name);
        fs::copy(source, destination).map_err(|error| error.to_string())?;
    }
    let current_path = current
        .strip_prefix(&root)
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let entries = list_directory(&root, &current)?;
    Ok(DirectoryListing {
        root_path: display_path(&root),
        current_path,
        entries,
    })
}

#[tauri::command]
fn read_preview(path: String, state: State<'_, LocalDirectory>) -> Result<PreviewData, String> {
    const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;
    let root = active_root(&state)?;
    let path = resolve_path(&root, &path)?;
    let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("The selected path is not a file".to_string());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let preview_limit = match file_kind(&extension) {
        "text" => 2 * 1024 * 1024,
        "document" => 16 * 1024 * 1024,
        _ => MAX_PREVIEW_BYTES,
    };
    if metadata.len() > preview_limit {
        return Err(format!(
            "This file exceeds the {} MB preview limit",
            preview_limit / 1024 / 1024
        ));
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    Ok(PreviewData {
        mime_type: mime_type(&extension).to_string(),
        data_base64: STANDARD.encode(&bytes),
        size_bytes: bytes.len() as u64,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(LocalDirectory::default())
        .manage(RemoteDirectory::default())
        .invoke_handler(tauri::generate_handler![
            default_directory,
            load_user_config,
            save_user_config,
            app_version,
            clear_directory,
            open_directory,
            connect_remote_directory,
            read_remote_directory,
            create_remote_directory,
            import_remote_files,
            read_remote_preview,
            read_directory,
            create_directory,
            import_files,
            read_preview
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{
        available_destination, file_kind, join_remote_path, list_directory, normalize_smb_endpoint,
        remote_absolute_path, resolve_path,
    };
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temporary_directory() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("r2-list-test-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn recognizes_preview_file_kinds() {
        assert_eq!(file_kind("png"), "image");
        assert_eq!(file_kind("mp3"), "audio");
        assert_eq!(file_kind("md"), "text");
        assert_eq!(file_kind("pdf"), "document");
    }

    #[test]
    fn refuses_paths_outside_the_selected_root() {
        let directory = temporary_directory();
        let root = directory.join("root");
        let outside = directory.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();

        assert!(resolve_path(&root, "../outside").is_err());
        assert!(resolve_path(&root, outside.to_str().unwrap()).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn keeps_remote_paths_within_the_share_root() {
        assert_eq!(
            remote_absolute_path("photos/2026").unwrap(),
            PathBuf::from("/photos/2026")
        );
        assert!(remote_absolute_path("../outside").is_err());
        assert!(remote_absolute_path(r"folder\outside").is_err());
    }

    #[test]
    fn normalizes_full_smb_urls_and_unc_paths() {
        assert_eq!(
            normalize_smb_endpoint("smb://nas.example.com/media", "").unwrap(),
            ("nas.example.com".to_string(), "media".to_string())
        );
        assert_eq!(
            normalize_smb_endpoint(r"\\nas.example.com\media", "").unwrap(),
            ("nas.example.com".to_string(), "media".to_string())
        );
        assert_eq!(
            normalize_smb_endpoint("nas.example.com", "media").unwrap(),
            ("nas.example.com".to_string(), "media".to_string())
        );
    }

    #[test]
    fn builds_cumulative_remote_paths_at_each_depth() {
        let first = join_remote_path("", "folder");
        let second = join_remote_path(&first, "nested");
        assert_eq!(first, "folder");
        assert_eq!(second, "folder/nested");
    }

    #[test]
    fn lists_real_file_metadata_relative_to_the_root() {
        let root = fs::canonicalize(temporary_directory()).unwrap();
        fs::write(root.join("notes.md"), "hello").unwrap();
        fs::create_dir(root.join("images")).unwrap();

        let entries = list_directory(&root, &root).unwrap();
        assert!(entries.iter().any(|entry| entry.path == "notes.md"
            && entry.kind == "text"
            && entry.size_bytes == 5));
        assert!(
            entries
                .iter()
                .any(|entry| entry.path == "images" && entry.kind == "directory")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chooses_a_new_name_instead_of_overwriting_imports() {
        let directory = temporary_directory();
        fs::write(directory.join("notes.txt"), "existing").unwrap();

        let destination = available_destination(&directory, "notes.txt");

        assert_eq!(destination.file_name().unwrap(), "notes (2).txt");
        fs::remove_dir_all(directory).unwrap();
    }
}
