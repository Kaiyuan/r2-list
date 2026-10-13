use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use dav_xml_client::{
    Resource,
    dav_xml::{FromXml, elements::Multistatus},
};
use reqwest::{Client as HttpClient, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use url::Url;

#[derive(Default)]
struct LocalDirectory(Mutex<Option<PathBuf>>);

#[derive(Default)]
struct RemoteDirectory(Mutex<Option<RemoteConnection>>);

struct RemoteConnection {
    id: String,
    client: WebDavClient,
}

#[derive(Clone)]
struct WebDavClient {
    http: HttpClient,
    endpoint: Url,
    username: String,
    password: String,
}

struct WebDavMetadata {
    is_collection: bool,
    size_bytes: u64,
}

impl WebDavClient {
    fn new(endpoint: &str, username: &str, password: &str) -> Result<Self, String> {
        let endpoint = if endpoint
            .get(..7)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"))
            || endpoint
                .get(..8)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
        {
            endpoint.to_string()
        } else {
            format!("https://{endpoint}")
        };
        let mut endpoint = Url::parse(&endpoint).map_err(|error| error.to_string())?;
        if !matches!(endpoint.scheme(), "http" | "https") {
            return Err("WebDAV address must use HTTP or HTTPS".to_string());
        }
        if endpoint.query().is_some() || endpoint.fragment().is_some() {
            return Err("WebDAV address cannot contain a query or fragment".to_string());
        }
        if endpoint.path().is_empty() {
            endpoint.set_path("/");
        }
        let http = HttpClient::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            http,
            endpoint,
            username: username.to_string(),
            password: password.to_string(),
        })
    }

    fn url_for(&self, relative: &str, collection: bool) -> Result<Url, String> {
        let relative = normalize_remote_relative_path(relative)?;
        let mut url = self.endpoint.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| "Invalid WebDAV address".to_string())?;
            segments.pop_if_empty();
            for component in relative
                .split('/')
                .filter(|component| !component.is_empty())
            {
                segments.push(component);
            }
            if collection {
                segments.push("");
            }
        }
        Ok(url)
    }

    fn request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.http
            .request(method, url)
            .basic_auth(&self.username, Some(&self.password))
    }

    async fn propfind(&self, relative: &str, depth: &str) -> Result<(Url, Vec<Resource>), String> {
        let url = self.url_for(relative, true)?;
        let response = self
            .request(Method::from_bytes(b"PROPFIND").map_err(|error| error.to_string())?, url.clone())
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body("<?xml version=\"1.0\" encoding=\"utf-8\"?><d:propfind xmlns:d=\"DAV:\"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>")
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let response = Self::ensure_success(response).await?;
        let body = response.bytes().await.map_err(|error| error.to_string())?;
        let multistatus =
            Multistatus::from_xml(body.to_vec()).map_err(|error| error.to_string())?;
        Ok((url, Resource::from_multistatus(&multistatus)))
    }

    async fn list_dir(&self, relative: &str) -> Result<Vec<Resource>, String> {
        let (request_url, resources) = self.propfind(relative, "1").await?;
        let request_path = request_url.path().trim_end_matches('/');
        Ok(resources
            .into_iter()
            .filter(|resource| {
                let candidate = request_url
                    .join(resource.path())
                    .map(|url| url.path().trim_end_matches('/').to_string());
                candidate.as_deref() != Ok(request_path)
            })
            .collect())
    }

    async fn stat(&self, relative: &str) -> Result<WebDavMetadata, String> {
        let (_, resources) = self.propfind(relative, "0").await?;
        let resource = resources
            .into_iter()
            .next()
            .ok_or_else(|| "WebDAV server returned no resource properties".to_string())?;
        Ok(WebDavMetadata {
            is_collection: resource.is_collection,
            size_bytes: resource.content_length.unwrap_or_default(),
        })
    }

    async fn exists(&self, relative: &str) -> Result<bool, String> {
        let url = self.url_for(relative, true)?;
        let response = self
            .request(Method::from_bytes(b"PROPFIND").map_err(|error| error.to_string())?, url)
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body("<?xml version=\"1.0\" encoding=\"utf-8\"?><d:propfind xmlns:d=\"DAV:\"><d:prop><d:resourcetype/></d:prop></d:propfind>")
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        Self::ensure_success(response).await?;
        Ok(true)
    }

    async fn create_dir(&self, relative: &str) -> Result<(), String> {
        let url = self.url_for(relative, true)?;
        let response = self
            .request(
                Method::from_bytes(b"MKCOL").map_err(|error| error.to_string())?,
                url,
            )
            .send()
            .await
            .map_err(|error| error.to_string())?;
        Self::ensure_success(response).await.map(|_| ())
    }

    async fn read_file(&self, relative: &str) -> Result<Vec<u8>, String> {
        let url = self.url_for(relative, false)?;
        let response = self
            .request(Method::GET, url)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        Self::ensure_success(response)
            .await?
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|error| error.to_string())
    }

    async fn write_file(&self, relative: &str, bytes: Vec<u8>) -> Result<(), String> {
        let url = self.url_for(relative, false)?;
        let response = self
            .request(Method::PUT, url)
            .body(bytes)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        Self::ensure_success(response).await.map(|_| ())
    }

    async fn ensure_success(response: Response) -> Result<Response, String> {
        let status = response.status();
        if status.is_success() {
            Ok(response)
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(if body.trim().is_empty() {
                format!("WebDAV request failed: HTTP {status}")
            } else {
                format!("WebDAV request failed: HTTP {status}: {body}")
            })
        }
    }
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
    username: String,
    #[serde(default)]
    password: String,
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

fn normalize_remote_relative_path(relative: &str) -> Result<String, String> {
    let relative = relative.replace('\\', "/");
    let mut components = Vec::new();
    for component in relative.split('/') {
        if component.is_empty() {
            continue;
        }
        if component == "." || component == ".." {
            return Err("Invalid remote path".to_string());
        }
        components.push(component);
    }
    Ok(components.join("/"))
}

fn remote_absolute_path(relative: &str) -> Result<PathBuf, String> {
    let relative = normalize_remote_relative_path(relative)?;
    if relative.is_empty() {
        Ok(PathBuf::from("/"))
    } else {
        Ok(PathBuf::from(format!("/{relative}")))
    }
}

fn remote_child_path(parent: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err("Invalid remote path".to_string());
    }
    let parent = parent.to_string_lossy();
    Ok(PathBuf::from(format!(
        "{}/{name}",
        parent.trim_end_matches('/')
    )))
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

async fn remote_listing(
    client: &WebDavClient,
    root_id: &str,
    current_path: &str,
) -> Result<DirectoryListing, String> {
    let current_path = normalize_remote_relative_path(current_path)?;
    let mut entries = client
        .list_dir(&current_path)
        .await?
        .into_iter()
        .map(|resource| {
            let name = resource.name().to_string();
            let relative_path = join_remote_path(&current_path, &name);
            let extension = Path::new(&name)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let modified_ms = resource
                .last_modified
                .map(SystemTime::from)
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|modified| modified.as_millis().min(u64::MAX as u128) as u64)
                .unwrap_or_default();
            DirectoryEntry {
                path: relative_path,
                name: name.clone(),
                kind: if resource.is_collection {
                    "directory"
                } else {
                    file_kind(&extension)
                }
                .to_string(),
                size_bytes: resource.content_length.unwrap_or_default(),
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
        current_path,
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
async fn connect_remote_directory(
    endpoint: String,
    username: String,
    password: String,
    state: State<'_, RemoteDirectory>,
    local: State<'_, LocalDirectory>,
) -> Result<DirectoryListing, String> {
    let endpoint = endpoint.trim().trim_end_matches('/').to_string();
    if endpoint.is_empty() {
        return Err("Enter a server address".to_string());
    }
    let client = WebDavClient::new(&endpoint, &username, &password)?;
    let id = format!("webdav://{endpoint}");
    let listing = remote_listing(&client, &id, "").await?;
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
async fn read_remote_directory(
    path: String,
    state: State<'_, RemoteDirectory>,
) -> Result<DirectoryListing, String> {
    let (id, client) = {
        let connection = state
            .0
            .lock()
            .map_err(|_| "Remote directory state unavailable".to_string())?;
        let connection = connection
            .as_ref()
            .ok_or_else(|| "Connect to a network location first".to_string())?;
        (connection.id.clone(), connection.client.clone())
    };
    remote_listing(&client, &id, &path).await
}

#[tauri::command]
async fn create_remote_directory(
    path: String,
    name: String,
    state: State<'_, RemoteDirectory>,
) -> Result<DirectoryListing, String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err("Enter a valid folder name".to_string());
    }
    let (id, client) = {
        let connection = state
            .0
            .lock()
            .map_err(|_| "Remote directory state unavailable".to_string())?;
        let connection = connection
            .as_ref()
            .ok_or_else(|| "Connect to a network location first".to_string())?;
        (connection.id.clone(), connection.client.clone())
    };
    let target = remote_child_path(&remote_absolute_path(&path)?, name)?;
    client.create_dir(&target.to_string_lossy()).await?;
    remote_listing(&client, &id, &path).await
}

#[tauri::command]
async fn import_remote_files(
    paths: Vec<String>,
    directory: String,
    state: State<'_, RemoteDirectory>,
) -> Result<DirectoryListing, String> {
    let (id, client) = {
        let connection = state
            .0
            .lock()
            .map_err(|_| "Remote directory state unavailable".to_string())?;
        let connection = connection
            .as_ref()
            .ok_or_else(|| "Connect to a network location first".to_string())?;
        (connection.id.clone(), connection.client.clone())
    };
    let target_directory = normalize_remote_relative_path(&directory)?;
    for source in paths {
        let source = fs::canonicalize(source).map_err(|error| error.to_string())?;
        if !source.is_file() {
            return Err("Only files can be added to this directory".to_string());
        }
        let name = source
            .file_name()
            .ok_or_else(|| "The selected file has no name".to_string())?;
        let name = name.to_string_lossy();
        let mut target = join_remote_path(&target_directory, &name);
        let mut suffix = 2;
        while client.exists(&target).await? {
            let stem = source.file_stem().unwrap_or_default().to_string_lossy();
            let extension = source.extension().map(|value| value.to_string_lossy());
            let renamed = if let Some(extension) = extension {
                format!("{stem} ({suffix}).{extension}")
            } else {
                format!("{stem} ({suffix})")
            };
            target = join_remote_path(&target_directory, &renamed);
            suffix += 1;
        }
        let bytes = fs::read(&source).map_err(|error| error.to_string())?;
        client.write_file(&target, bytes).await?;
    }
    remote_listing(&client, &id, &directory).await
}

#[tauri::command]
async fn read_remote_preview(
    path: String,
    state: State<'_, RemoteDirectory>,
) -> Result<PreviewData, String> {
    const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;
    let client = {
        let connection = state
            .0
            .lock()
            .map_err(|_| "Remote directory state unavailable".to_string())?;
        connection
            .as_ref()
            .ok_or_else(|| "Connect to a network location first".to_string())?
            .client
            .clone()
    };
    let path = normalize_remote_relative_path(&path)?;
    let metadata = client.stat(&path).await?;
    if metadata.is_collection {
        return Err("The selected path is not a file".to_string());
    }
    let extension = Path::new(&path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let preview_limit = match file_kind(&extension) {
        "text" => 2 * 1024 * 1024,
        "document" => 16 * 1024 * 1024,
        _ => MAX_PREVIEW_BYTES,
    };
    if metadata.size_bytes > preview_limit {
        return Err(format!(
            "This file exceeds the {} MB preview limit",
            preview_limit / 1024 / 1024
        ));
    }
    let bytes = client.read_file(&path).await?;
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
        WebDavClient, available_destination, file_kind, join_remote_path, list_directory,
        remote_absolute_path, remote_child_path, resolve_path,
    };
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        thread,
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
        assert_eq!(
            remote_absolute_path(r"photos\2026\nested").unwrap(),
            PathBuf::from("/photos/2026/nested")
        );
        assert!(remote_absolute_path("../outside").is_err());
        assert!(remote_absolute_path(r"photos\..\outside").is_err());
    }

    #[test]
    fn keeps_remote_paths_posix_style_at_each_depth() {
        assert_eq!(
            remote_absolute_path("photos/2026")
                .unwrap()
                .to_string_lossy(),
            "/photos/2026"
        );
        assert_eq!(
            remote_child_path(&remote_absolute_path("photos/2026").unwrap(), "nested")
                .unwrap()
                .to_string_lossy(),
            "/photos/2026/nested"
        );
    }

    #[test]
    fn builds_nested_webdav_urls_beneath_the_configured_endpoint() {
        let client =
            WebDavClient::new("http://dav.example.test/dav/files/user/", "alice", "secret")
                .unwrap();

        let url = client
            .url_for(r"folder with space\nested\third", true)
            .unwrap();

        assert_eq!(
            url.as_str(),
            "http://dav.example.test/dav/files/user/folder%20with%20space/nested/third/"
        );
    }

    #[tokio::test]
    async fn lists_third_level_webdav_directories_using_the_endpoint_base_path() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let xml = r#"<?xml version="1.0" encoding="utf-8"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>/dav/files/user/one/two/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response><d:response><d:href>/dav/files/user/one/two/three/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let response_body = xml.to_string();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]).to_string();
            write!(
                stream,
                "HTTP/1.1 207 Multi-Status\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .unwrap();
            request
        });
        let client = WebDavClient::new(
            &format!("http://{address}/dav/files/user"),
            "alice",
            "secret",
        )
        .unwrap();

        let entries = client.list_dir("one/two").await.unwrap();
        let request = server.join().unwrap();

        assert!(request.starts_with("PROPFIND /dav/files/user/one/two/ HTTP/1.1"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name(), "three");
        assert!(entries[0].is_collection);
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
