import { useEffect, useState, type CSSProperties, type FormEvent, type MouseEvent } from "react";
import {
  Archive, ArrowDownToLine, ArrowLeft, Check, ChevronDown, ChevronRight, ClipboardPaste, Cloud, Copy, Globe, LayoutGrid,
  File, FileAudio, FileCode2, FileImage, FileText, Film, Folder, FolderPlus, HardDrive, Image, MoreHorizontal, Move,
  Languages, List, Moon, Pencil, Plus, RefreshCw, Search, Settings2, ShieldCheck, Sun, Trash2, Upload, X,
} from "lucide-react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { buildObjectUrl, formatLinks, type ShareFormat, type UrlRules } from "./lib/urlMapping";
import { detectLocale, formatItemCount, isLocale, locales, translate, type Locale, type TranslationKey } from "./i18n";
import { resolveTheme, type ThemePreference } from "./theme";
import { getVersionStatus } from "./version";
import { buildRemoteConfig, buildRemoteLocationLabel } from "./networkConfig";
import { PreviewContent } from "./PreviewContent";
import "./App.css";
import "./preferences.css";

type EntryKind = "directory" | "image" | "video" | "audio" | "text" | "document" | "archive" | "file";
type Entry = { path: string; name: string; kind: EntryKind; sizeBytes: number; modifiedMs: number; extension: string };
type DirectoryListing = { rootPath: string; currentPath: string; entries: Entry[] };
type PreviewData = { mimeType: string; dataBase64: string; sizeBytes: number };
type DirectoryConfig = { path: string; rules: UrlRules; kind?: "local" | "webdav"; label?: string; endpoint?: string; username?: string; password?: string };
type NetworkDraft = { endpoint: string; username: string; password: string };
type LocationDraft = { label: string; rules: UrlRules; endpoint: string; username: string; password: string };
type ConfigLocation = { directory: string; configPath: string };
type TransferClipboard = { mode: "copy" | "move"; entry: Entry };
type UserConfig = { directories: DirectoryConfig[]; activeDirectory: string | null; locale: Locale | null; theme: ThemePreference | null };
type MenuState = { x: number; y: number; entry: Entry; preview?: boolean } | null;
type UpdateStatus = { status: "checking" | "up-to-date" | "update-available" | "error"; latest: string | null; message: string };
const initialRules: UrlRules = { baseUrl: "https://cdn.studio-example.com", prefix: "northstar/media", template: "" };
const GITHUB_REPO_URL = "https://github.com/Kaiyuan/r2-list";
const GITHUB_API_URL = "https://api.github.com/repos/Kaiyuan/r2-list";
const GITHUB_RELEASES_URL = "https://github.com/Kaiyuan/r2-list/releases/latest";
const parentPath = (path: string) => path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
const formatBytes = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = -1;
  do { value /= 1024; unit += 1; } while (value >= 1024 && unit < units.length - 1);
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(value)} ${units[unit]}`;
};

function loadLocale(): Locale {
  const saved = localStorage.getItem("r2-list-language");
  return isLocale(saved) ? saved : detectLocale(navigator.language);
}

function loadThemePreference(): ThemePreference {
  const saved = localStorage.getItem("r2-list-theme-mode");
  if (saved === "system" || saved === "light" || saved === "dark") return saved;
  return "system";
}

function formatModified(value: number, locale: Locale): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat(locales.find((item) => item.id === locale)?.htmlLang, {
    dateStyle: "medium", timeStyle: "short",
  }).format(new Date(value));
}

function EntryIcon({ kind, size = 18 }: { kind: EntryKind; size?: number }) {
  if (kind === "directory") return <Folder size={size} className="entry-icon folder-icon" />;
  if (kind === "image") return <FileImage size={size} className="entry-icon image-icon" />;
  if (kind === "video") return <Film size={size} className="entry-icon video-icon" />;
  if (kind === "audio") return <FileAudio size={size} className="entry-icon audio-icon" />;
  if (kind === "text") return <FileCode2 size={size} className="entry-icon document-icon" />;
  if (kind === "document") return <FileText size={size} className="entry-icon document-icon" />;
  if (kind === "archive") return <Archive size={size} className="entry-icon archive-icon" />;
  return <File size={size} className="entry-icon" />;
}

function RemoteThumbnail({ path }: { path: string }) {
  const [url, setUrl] = useState("");
  useEffect(() => {
    let active = true;
    let objectUrl = "";
    void invoke<PreviewData>("read_remote_preview", { path }).then((preview) => {
      const binary = atob(preview.dataBase64);
      const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
      objectUrl = URL.createObjectURL(new Blob([bytes.buffer as ArrayBuffer], { type: preview.mimeType }));
      if (active) setUrl(objectUrl);
    }).catch(() => undefined);
    return () => {
      active = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [path]);
  return url ? <img src={url} alt="" loading="lazy" /> : null;
}

function App() {
  const [entries, setEntries] = useState<Entry[]>([]);
  const [rootPath, setRootPath] = useState("");
  const [currentPath, setCurrentPath] = useState("");
  const [selectedPath, setSelectedPath] = useState("");
  const [loadingDirectory, setLoadingDirectory] = useState(true);
  const [search, setSearch] = useState("");
  const [viewMode, setViewMode] = useState<"list" | "thumbnails">("list");
  const [locationMenuOpen, setLocationMenuOpen] = useState(false);
  const [networkOpen, setNetworkOpen] = useState(false);
  const [networkDraft, setNetworkDraft] = useState<NetworkDraft>({ endpoint: "", username: "", password: "" });
  const [directoryConfigs, setDirectoryConfigs] = useState<DirectoryConfig[]>([]);
  const [activeDirectory, setActiveDirectory] = useState("");
  const [configLoaded, setConfigLoaded] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [locationDraft, setLocationDraft] = useState<LocationDraft>({ label: "", rules: initialRules, endpoint: "", username: "", password: "" });
  const [editingDirectory, setEditingDirectory] = useState<string | null>(null);
  const [configLocation, setConfigLocation] = useState<ConfigLocation | null>(null);
  const [configSaving, setConfigSaving] = useState(false);
  const [githubStatus, setGithubStatus] = useState<"idle" | "checking" | "ok" | "error">("idle");
  const [versionCheckId, setVersionCheckId] = useState(0);
  const [appVersion, setAppVersion] = useState("");
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>({ status: "checking", latest: null, message: "" });
  const [deleteTarget, setDeleteTarget] = useState<Entry | null>(null);
  const [transfer, setTransfer] = useState<TransferClipboard | null>(null);
  const [newFolderOpen, setNewFolderOpen] = useState(false);
  const [newFolderName, setNewFolderName] = useState("");
  const [locale, setLocale] = useState<Locale>(loadLocale);
  const [themePreference, setThemePreference] = useState<ThemePreference>(loadThemePreference);
  const [systemDark, setSystemDark] = useState(() => window.matchMedia("(prefers-color-scheme: dark)").matches);
  const [menu, setMenu] = useState<MenuState>(null);
  const [previewEntry, setPreviewEntry] = useState<Entry | null>(null);
  const [previewBytes, setPreviewBytes] = useState<Uint8Array | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState("");
  const [toast, setToast] = useState("");
  const theme = resolveTheme(themePreference, systemDark);
  const rules = directoryConfigs.find((directory) => directory.path === rootPath)?.rules ?? initialRules;
  const editingConfig = directoryConfigs.find((directory) => directory.path === editingDirectory);
  const t = (key: TranslationKey, values?: Record<string, string | number>) => translate(locale, key, values);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.lang = locales.find((item) => item.id === locale)?.htmlLang ?? "en";
    localStorage.setItem("r2-list-theme-mode", themePreference);
    localStorage.setItem("r2-list-language", locale);
  }, [locale, theme, themePreference]);
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const updateSystemTheme = () => setSystemDark(media.matches);
    updateSystemTheme();
    media.addEventListener("change", updateSystemTheme);
    return () => media.removeEventListener("change", updateSystemTheme);
  }, []);
  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const savedConfig = await invoke<UserConfig>("load_user_config");
        if (!active) return;
        const migratedRules = (() => {
          try { return { ...initialRules, ...JSON.parse(localStorage.getItem("r2-list-url-rules") ?? "{}") as Partial<UrlRules> }; }
          catch { return initialRules; }
        })();
        const savedDirectories = savedConfig.directories?.filter((directory) => String(directory.kind) !== "smb") ?? [];
        const directories = savedDirectories.length
          ? savedDirectories
          : (localStorage.getItem("r2-list-local-directory") ? [{ path: localStorage.getItem("r2-list-local-directory")!, rules: migratedRules }] : []);
        setDirectoryConfigs(directories);
        if (isLocale(savedConfig.locale)) setLocale(savedConfig.locale);
        if (savedConfig.theme === "system" || savedConfig.theme === "light" || savedConfig.theme === "dark") setThemePreference(savedConfig.theme);
        const savedPath = directories.some((directory) => directory.path === savedConfig.activeDirectory)
          ? savedConfig.activeDirectory!
          : directories[0]?.path ?? await invoke<string | null>("default_directory");
        if (savedPath) await loadDirectory(savedPath, directories);
        setConfigLoaded(true);
      } catch (error) {
        if (active) setToast(`${t("directoryLoadFailed")}: ${String(error)}`);
        const fallbackPath = localStorage.getItem("r2-list-local-directory") ?? await invoke<string | null>("default_directory").catch(() => null);
        if (active && fallbackPath) await loadDirectory(fallbackPath);
        if (active) setConfigLoaded(true);
      } finally {
        if (active) setLoadingDirectory(false);
      }
    })();
    return () => { active = false; };
  }, []);
  useEffect(() => {
    if (!configLoaded) return;
    const config: UserConfig = { directories: directoryConfigs, activeDirectory: activeDirectory || null, locale, theme: themePreference };
    void invoke("save_user_config", { config }).catch((error: unknown) => console.error("Could not save user config", error));
  }, [configLoaded, directoryConfigs, activeDirectory, locale, themePreference]);
  useEffect(() => {
    void invoke<string>("app_version").then(setAppVersion).catch(() => setAppVersion(""));
  }, []);
  useEffect(() => {
    if (!settingsOpen) return;
    void invoke<ConfigLocation>("config_location")
      .then(setConfigLocation)
      .catch(() => setConfigLocation(null));
  }, [settingsOpen]);
  useEffect(() => {
    if (!settingsOpen) return;
    if (!appVersion) {
      setUpdateStatus({ status: "error", latest: null, message: t("versionCheckFailed") });
      return;
    }

    let active = true;
    const controller = new AbortController();
    setUpdateStatus({ status: "checking", latest: null, message: t("checkingUpdates") });

    void fetch(`${GITHUB_API_URL}/releases/latest`, {
      signal: controller.signal,
      headers: {
        Accept: "application/vnd.github+json",
        "User-Agent": "R2-List-App",
      },
    }).then(async (response) => {
      if (!response.ok) throw new Error(`${response.status}`);
      const payload = await response.json() as { tag_name?: string };
      if (!payload.tag_name) throw new Error("missing tag");
      return payload.tag_name;
    }).then((latestTag) => {
      if (!active) return;
      const latestVersion = latestTag.replace(/^v/i, "");
      const status = getVersionStatus(appVersion, latestVersion);
      if (status.status === "update-available") {
        setUpdateStatus({
          status: "update-available",
          latest: status.latest,
          message: `${t("updateAvailable")}: ${status.latest}`,
        });
        return;
      }
      setUpdateStatus({
        status: "up-to-date",
        latest: status.latest,
        message: t("upToDate"),
      });
    }).catch(() => {
      if (!active) return;
      setUpdateStatus({ status: "error", latest: null, message: t("versionCheckFailed") });
    });

    return () => {
      active = false;
      controller.abort();
    };
  }, [settingsOpen, appVersion, locale, versionCheckId]);
  useEffect(() => {
    if (!toast) return;
    const timeout = window.setTimeout(() => setToast(""), 2600);
    return () => window.clearTimeout(timeout);
  }, [toast]);
  useEffect(() => {
    if (!previewEntry) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setPreviewEntry(null);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [previewEntry]);
  useEffect(() => {
    const target = entries.find((entry) => entry.path === selectedPath);
    if (!target || !rootPath) {
      setPreviewBytes(null);
      setPreviewUrl(null);
      setPreviewLoading(false);
      return;
    }
    let active = true;
    let objectUrl: string | null = null;
    setPreviewBytes(null);
    setPreviewUrl(null);
    setPreviewError("");
    setPreviewLoading(true);
    const isRemote = directoryConfigs.some((directory) => directory.path === activeDirectory && directory.kind && directory.kind !== "local");
    if (isRemote) {
      if (target.kind === "file" || target.kind === "archive") {
        setPreviewLoading(false);
        return () => { active = false; };
      }
      void invoke<PreviewData>("read_remote_preview", { path: target.path }).then((result) => {
        const binary = atob(result.dataBase64);
        const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
        objectUrl = URL.createObjectURL(new Blob([bytes.buffer as ArrayBuffer], { type: result.mimeType }));
        if (active) {
          setPreviewBytes(bytes);
          setPreviewUrl(objectUrl);
        }
      }).catch((error: unknown) => {
        if (active) setPreviewError(`${t("previewLoadFailed")}: ${String(error)}`);
      }).finally(() => {
        if (active) setPreviewLoading(false);
      });
      return () => {
        active = false;
        if (objectUrl) URL.revokeObjectURL(objectUrl);
      };
    }
    const pathSeparator = rootPath.includes("\\") ? "\\" : "/";
    const separator = rootPath.endsWith("\\") || rootPath.endsWith("/") ? "" : pathSeparator;
    const absolutePath = `${rootPath}${separator}${target.path.replace(/\//g, "\\")}`;
    if (["image", "video", "audio"].includes(target.kind) || target.extension === "pdf") {
      setPreviewUrl(convertFileSrc(absolutePath));
      setPreviewLoading(false);
      return () => { active = false; };
    }
    if (target.kind === "file" || target.kind === "archive") {
      setPreviewLoading(false);
      return () => { active = false; };
    }
    void invoke<PreviewData>("read_preview", { path: target.path }).then((result) => {
      const binary = atob(result.dataBase64);
      const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
      objectUrl = URL.createObjectURL(new Blob([bytes.buffer as ArrayBuffer], { type: result.mimeType }));
      if (active) {
        setPreviewBytes(bytes);
        setPreviewUrl(objectUrl);
      }
    }).catch((error: unknown) => {
      if (active) setPreviewError(`${t("previewLoadFailed")}: ${String(error)}`);
    }).finally(() => {
      if (active) setPreviewLoading(false);
    });
    return () => {
      active = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [selectedPath, entries, rootPath, directoryConfigs, activeDirectory]);

  const visibleEntries = entries
    .filter((entry) => entry.name.toLowerCase().includes(search.toLowerCase()))
    .sort((a, b) => Number(b.kind === "directory") - Number(a.kind === "directory") || a.name.localeCompare(b.name));
  const selectedEntry = entries.find((entry) => entry.path === selectedPath);
  const pathParts = currentPath ? currentPath.split("/") : [];

  function isRemoteDirectory() {
    return directoryConfigs.some((directory) => directory.path === activeDirectory && directory.kind && directory.kind !== "local");
  }
  function openLocationEditor(directory: DirectoryConfig) {
    setEditingDirectory(directory.path);
    setLocationDraft({
      label: directory.label ?? "",
      rules: directory.rules ?? initialRules,
      endpoint: directory.endpoint ?? "",
      username: directory.username ?? "",
      password: directory.password ?? "",
    });
    setLocationMenuOpen(false);
    setMenu(null);
  }
  function getThumbnailUrl(entry: Entry): string {
    const pathSeparator = rootPath.includes("\\") ? "\\" : "/";
    const separator = rootPath.endsWith("\\") || rootPath.endsWith("/") ? "" : pathSeparator;
    return convertFileSrc(`${rootPath}${separator}${entry.path.replace(/\//g, pathSeparator)}`);
  }
  function openNetworkForm(config?: DirectoryConfig) {
    setNetworkDraft({
      endpoint: config?.endpoint ?? "",
      username: config?.username ?? "",
      password: config?.password ?? "",
    });
    setLocationMenuOpen(false);
    setNetworkOpen(true);
  }
  async function connectRemoteWithConfig(connection: NetworkDraft) {
    const config = buildRemoteConfig(connection);
    setLoadingDirectory(true);
    try {
      const listing = await invoke<DirectoryListing>("connect_remote_directory", {
        endpoint: config.endpoint,
        username: config.username,
        password: config.password,
      });
      setRootPath(listing.rootPath);
      setActiveDirectory(listing.rootPath);
      setCurrentPath(listing.currentPath);
      setEntries(listing.entries);
      setSelectedPath(listing.entries.find((entry) => entry.kind !== "directory")?.path ?? "");
      setSearch("");
      const label = buildRemoteLocationLabel(config.endpoint);
      setDirectoryConfigs((current) => current.some((directory) => directory.path === listing.rootPath)
        ? current.map((directory) => directory.path === listing.rootPath ? {
            ...directory,
            kind: "webdav",
            label: directory.label || label,
            endpoint: config.endpoint,
            username: config.username,
            password: config.password,
          } : directory)
        : [...current, {
            path: listing.rootPath,
            kind: "webdav",
            label,
            endpoint: config.endpoint,
            username: config.username,
            password: config.password,
            rules: initialRules,
          }]);
      setNetworkOpen(false);
      setNetworkDraft((draft) => ({ ...draft, password: config.password }));
      setToast(t("networkConnected"));
    } catch (error) {
      setToast(`${t("directoryLoadFailed")}: ${String(error)}`);
    } finally {
      setLoadingDirectory(false);
    }
  }
  function openGithubProject() {
    void openUrl(GITHUB_REPO_URL);
  }
  async function loadDirectory(path: string, configs: DirectoryConfig[] = directoryConfigs) {
    const savedNetwork = configs.find((directory) => directory.path === path && directory.kind === "webdav");
    if (savedNetwork) {
      await connectRemoteWithConfig({
        endpoint: savedNetwork.endpoint ?? "",
        username: savedNetwork.username ?? "",
        password: savedNetwork.password ?? "",
      });
      return;
    }
    setLoadingDirectory(true);
    try {
      const listing = await invoke<DirectoryListing>("open_directory", { path });
      setRootPath(listing.rootPath);
      setActiveDirectory(listing.rootPath);
      setDirectoryConfigs((current) => current.some((directory) => directory.path === listing.rootPath)
        ? current.map((directory) => directory.path === listing.rootPath ? { ...directory, kind: "local" } : directory)
        : [...current, { path: listing.rootPath, kind: "local", label: listing.rootPath.split(/[\\/]/).filter(Boolean).pop() ?? listing.rootPath, rules: initialRules }]);
      setCurrentPath(listing.currentPath);
      setEntries(listing.entries);
      setSelectedPath(listing.entries.find((entry) => entry.kind !== "directory")?.path ?? "");
      setSearch("");
      localStorage.setItem("r2-list-local-directory", listing.rootPath);
    } catch (error) {
      setToast(`${t("directoryLoadFailed")}: ${String(error)}`);
    } finally {
      setLoadingDirectory(false);
    }
  }
  async function chooseDirectory() {
    try {
      const selected = await openDialog({ directory: true, multiple: false });
      if (typeof selected === "string") await loadDirectory(selected);
    } catch (error) { setToast(`${t("directoryLoadFailed")}: ${String(error)}`); }
  }
  async function connectRemote(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    await connectRemoteWithConfig(networkDraft);
  }
  async function removeDirectory(path: string) {
    const remaining = directoryConfigs.filter((directory) => directory.path !== path);
    setDirectoryConfigs(remaining);
    if (activeDirectory !== path) return;
    const nextPath = remaining[0]?.path;
    if (nextPath) await loadDirectory(nextPath);
    else {
      await invoke("clear_directory").catch(() => undefined);
      setActiveDirectory("");
      setRootPath("");
      setCurrentPath("");
      setEntries([]);
      setSelectedPath("");
      setPreviewEntry(null);
    }
  }
  async function navigateTo(path: string) {
    setLoadingDirectory(true);
    try {
      const isRemote = directoryConfigs.some((directory) => directory.path === activeDirectory && directory.kind && directory.kind !== "local");
      const listing = await invoke<DirectoryListing>(isRemote ? "read_remote_directory" : "read_directory", { path });
      setCurrentPath(listing.currentPath);
      setEntries(listing.entries);
      setSelectedPath(listing.entries.find((entry) => entry.kind !== "directory")?.path ?? "");
      setSearch("");
    } catch (error) {
      setToast(`${t("directoryLoadFailed")}: ${String(error)}`);
    } finally { setLoadingDirectory(false); }
  }
  async function chooseFiles() {
    try {
      const selected = await openDialog({ multiple: true });
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      const isRemote = directoryConfigs.some((directory) => directory.path === activeDirectory && directory.kind && directory.kind !== "local");
      const listing = await invoke<DirectoryListing>(isRemote ? "import_remote_files" : "import_files", { paths, directory: currentPath });
      setEntries(listing.entries);
      setToast(t("filesAdded", { count: paths.length }));
    } catch (error) { setToast(`${t("directoryLoadFailed")}: ${String(error)}`); }
  }
  async function createFolder(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      const isRemote = directoryConfigs.some((directory) => directory.path === activeDirectory && directory.kind && directory.kind !== "local");
      const listing = await invoke<DirectoryListing>(isRemote ? "create_remote_directory" : "create_directory", { path: currentPath, name: newFolderName });
      setEntries(listing.entries);
      setNewFolderName("");
      setNewFolderOpen(false);
      setToast(t("folderCreated"));
    } catch (error) { setToast(`${t("directoryLoadFailed")}: ${String(error)}`); }
  }
  function openEntry(entry: Entry) {
    if (entry.kind === "directory") {
      void navigateTo(entry.path);
      return;
    }
    setSelectedPath(entry.path);
  }
  function openContextMenu(event: MouseEvent, entry: Entry) {
    event.preventDefault();
    setSelectedPath(entry.path);
    setMenu({ x: Math.min(event.clientX, window.innerWidth - 236), y: Math.min(event.clientY, window.innerHeight - 340), entry });
  }
  function openPreviewContextMenu(event: MouseEvent, entry: Entry) {
    event.preventDefault();
    event.stopPropagation();
    setMenu({ x: Math.min(event.clientX, window.innerWidth - 236), y: Math.min(event.clientY, window.innerHeight - 200), entry, preview: true });
  }
  function openContextMenuForButton(entry: Entry) {
    setMenu({ x: Math.max(16, window.innerWidth - 438), y: Math.max(80, window.innerHeight - 310), entry });
  }
  async function copyLinks(entry: Entry, format: ShareFormat) {
    const targets = entry.kind === "directory"
      ? entries.filter((candidate) => candidate.kind !== "directory" && candidate.path.startsWith(`${entry.path}/`))
      : [entry];
    const links = targets.map((target) => ({ name: target.name, url: buildObjectUrl(target.path, rules) }));
    if (!links.length) { setToast(t("emptyFolder")); return; }
    try {
      const formatted = formatLinks(links, format);
      if (format === "html" && navigator.clipboard.write && typeof ClipboardItem !== "undefined") {
        await navigator.clipboard.write([new ClipboardItem({
          "text/html": new Blob([formatted], { type: "text/html" }),
          "text/plain": new Blob([formatLinks(links, "url")], { type: "text/plain" }),
        })]);
      } else {
        await navigator.clipboard.writeText(formatted);
      }
      setMenu(null);
      const formatLabel = format === "url" ? t("formatUrl") : format === "markdown" ? t("formatMarkdown") : t("formatHtml");
      setToast(t("copiedLinks", { count: links.length, format: formatLabel }));
    } catch { setToast(t("clipboardUnavailable")); }
  }
  async function saveLocation(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      buildObjectUrl("sample-check.txt", locationDraft.rules);
    } catch {
      setToast(t("invalidRules"));
      return;
    }
    const targetPath = editingDirectory;
    const updatedConfigs = directoryConfigs.map((directory) => directory.path === targetPath ? {
      ...directory,
      label: locationDraft.label.trim() || directory.label,
      rules: locationDraft.rules,
      ...(directory.kind === "webdav" ? {
        endpoint: locationDraft.endpoint.trim(),
        username: locationDraft.username.trim(),
        password: locationDraft.password,
      } : {}),
    } : directory);
    setDirectoryConfigs(updatedConfigs);
    setEditingDirectory(null);
    setToast(t("locationSaved"));
    if (targetPath && targetPath === activeDirectory) {
      const updated = updatedConfigs.find((directory) => directory.path === targetPath);
      if (updated?.kind === "webdav") await loadDirectory(targetPath, updatedConfigs);
    }
  }
  function safePreview(path: string) {
    try { return buildObjectUrl(path, locationDraft.rules); }
    catch { return t("invalidRules"); }
  }
  function startTransfer(entry: Entry, mode: "copy" | "move") {
    setTransfer({ mode, entry });
    setMenu(null);
    setToast(t(mode === "copy" ? "copyPending" : "movePending", { name: entry.name }));
  }
  async function pasteInto(targetDirectory: string) {
    if (!transfer) return;
    const { entry, mode } = transfer;
    setMenu(null);
    try {
      const remote = isRemoteDirectory();
      const command = remote
        ? (mode === "copy" ? "copy_remote_entry" : "move_remote_entry")
        : (mode === "copy" ? "copy_entry" : "move_entry");
      await invoke(command, { source: entry.path, destinationDirectory: targetDirectory });
      setTransfer(null);
      await navigateTo(currentPath);
      setToast(t(mode === "copy" ? "copiedItem" : "movedItem", { name: entry.name }));
    } catch (error) {
      setToast(`${t("operationFailed")}: ${String(error)}`);
    }
  }
  async function confirmDelete() {
    const target = deleteTarget;
    if (!target) return;
    try {
      await invoke(isRemoteDirectory() ? "delete_remote_entry" : "delete_entry", { path: target.path });
      setDeleteTarget(null);
      if (previewEntry?.path === target.path) setPreviewEntry(null);
      if (selectedPath === target.path) setSelectedPath("");
      await navigateTo(currentPath);
      setToast(t("deletedItem", { name: target.name }));
    } catch (error) {
      setDeleteTarget(null);
      setToast(`${t("operationFailed")}: ${String(error)}`);
    }
  }
  async function chooseConfigDirectory() {
    try {
      const selected = await openDialog({ directory: true, multiple: false });
      if (typeof selected !== "string") return;
      setConfigSaving(true);
      const location = await invoke<ConfigLocation>("set_config_directory", { directory: selected });
      setConfigLocation(location);
      setToast(t("configDirectorySaved"));
    } catch (error) {
      setToast(`${t("operationFailed")}: ${String(error)}`);
    } finally {
      setConfigSaving(false);
    }
  }
  async function resetConfigDirectory() {
    try {
      setConfigSaving(true);
      const location = await invoke<ConfigLocation>("set_config_directory", { directory: "" });
      setConfigLocation(location);
      setToast(t("configDirectorySaved"));
    } catch (error) {
      setToast(`${t("operationFailed")}: ${String(error)}`);
    } finally {
      setConfigSaving(false);
    }
  }
  async function testGithubConnection() {
    setGithubStatus("checking");
    try {
      const response = await fetch(GITHUB_API_URL, {
        headers: { Accept: "application/vnd.github+json", "User-Agent": "R2-List-App" },
      });
      setGithubStatus(response.ok ? "ok" : "error");
    } catch {
      setGithubStatus("error");
    }
  }

  return (
    <main className="app-shell" onClick={() => menu && setMenu(null)}>
      <aside className="sidebar">
        <div className="brand-lockup"><div className="brand-mark"><Cloud size={19} strokeWidth={2.3} /></div><div><strong>R2 List</strong><span>{t("objectWorkspace")}</span></div><button className="icon-button sidebar-more" aria-label={t("settings")} title={t("settings")} onClick={() => setSettingsOpen(true)}><Settings2 size={18} /></button></div>
        <div className="sidebar-section"><div className="section-label">{t("workspace")}</div><button className="nav-item active"><HardDrive size={17} /><span>{t("files")}</span><span className="nav-count">{entries.length}</span></button><button className="nav-item" onClick={() => setSettingsOpen(true)}><Settings2 size={17} /><span>{t("settings")}</span></button></div>
        <div className="sidebar-section locations-section"><div className="section-heading"><span className="section-label">{t("locations")}</span><button className="icon-button tiny" aria-label={t("addLocation")} title={t("addLocation")} onClick={() => setLocationMenuOpen((open) => !open)}><Plus size={15} /></button></div>
          {locationMenuOpen && <div className="location-add-menu"><button onClick={() => { setLocationMenuOpen(false); void chooseDirectory(); }}><HardDrive size={16} /><span>{t("localFolderOption")}</span></button><button onClick={() => openNetworkForm()}><Cloud size={16} /><span>{t("webdavOption")}</span></button></div>}
          {directoryConfigs.map((directory) => <div className={`location-entry ${directory.path === activeDirectory ? "active" : ""}`} key={directory.path}><button className="location-item" onClick={() => void loadDirectory(directory.path)}><span className={`location-dot ${directory.kind && directory.kind !== "local" ? "remote-dot" : "local-dot"}`} /><span className="location-copy"><strong>{directory.label || directory.path.split(/[\\/]/).filter(Boolean).pop()}</strong><small>{directory.kind && directory.kind !== "local" ? directory.kind.toUpperCase() : directory.path}</small></span></button><button className="location-action edit-action" aria-label={t("editLocation")} title={t("editLocation")} onClick={() => openLocationEditor(directory)}><Pencil size={15} /></button><button className="location-action" aria-label={t("removeLocation")} title={t("removeLocation")} onClick={() => void removeDirectory(directory.path)}><Trash2 size={15} /></button></div>)}
          <button className="add-location" onClick={() => setLocationMenuOpen((open) => !open)}><Plus size={14} /> {t("addLocation")}</button>
        </div>
        <div className="sidebar-bottom"><div className="sync-note"><span className="note-icon"><ShieldCheck size={15} /></span><span><strong>{t("urlOnly")}</strong><small>{t("externalSync")}</small></span></div>
          <div className="profile-button"><span className="avatar">R</span><span className="profile-copy"><strong>R2 List</strong><small>{t("personalWorkspace")}</small></span></div>
        </div>
      </aside>

      <section className="content-area">
        <header className="topbar"><div className="breadcrumbs"><button className="breadcrumb-root" onClick={() => void navigateTo("")}>{rootPath.split(/[\\/]/).filter(Boolean).pop() ?? t("mediaLibrary")}</button>{pathParts.map((part, index) => { const targetPath = pathParts.slice(0, index + 1).join("/"); return <span className="breadcrumb-part" key={targetPath}><ChevronRight size={13} /><button onClick={() => void navigateTo(targetPath)}>{part}</button></span>; })}</div>
          <div className="topbar-actions"><span className="connection-state"><span />{t("local")}</span><label className="preference-select" title={t("language")}><Languages size={14} /><select aria-label={t("language")} value={locale} onChange={(event) => { if (isLocale(event.currentTarget.value)) setLocale(event.currentTarget.value); }}>{locales.map((item) => <option value={item.id} key={item.id}>{item.label}</option>)}</select><ChevronDown size={12} /></label><label className="preference-select theme-select" title={t("theme")}>{theme === "dark" ? <Moon size={14} /> : <Sun size={14} />}<select aria-label={t("theme")} value={themePreference} onChange={(event) => setThemePreference(event.currentTarget.value as ThemePreference)}><option value="system">{t("themeSystem")}</option><option value="light">{t("themeLight")}</option><option value="dark">{t("themeDark")}</option></select><ChevronDown size={12} /></label><button className="icon-button" aria-label={t("settings")} title={t("settings")} onClick={() => setSettingsOpen(true)}><Settings2 size={17} /></button></div>
        </header>
        <div className="workspace-heading"><div><div className="eyebrow"><span className="eyebrow-line" />{t("localDirectory")} <span className="muted-separator">/</span> {t("urlMapped")}</div><h1>{pathParts[pathParts.length - 1] ?? rootPath.split(/[\\/]/).filter(Boolean).pop() ?? t("mediaLibrary")}</h1><p className="workspace-path"><HardDrive size={13} /> {rootPath || t("chooseFolder")} <span className="path-dot">·</span> {formatItemCount(locale, visibleEntries.length)}</p></div>
          <div className="heading-actions"><button className="button-primary" onClick={() => void chooseFiles()}><Upload size={15} /> {t("addFiles")}</button></div>
        </div>
        <div className="toolbar"><div className="search-box"><Search size={16} /><input aria-label={t("searchAria")} value={search} onChange={(event) => setSearch(event.currentTarget.value)} placeholder={t("searchPlaceholder")} /><kbd>⌘ F</kbd></div><div className="toolbar-spacer" /><div className="view-toggle" role="group" aria-label={t("viewOptions")}><button className={`icon-button ${viewMode === "list" ? "active" : ""}`} aria-label={t("listView")} title={t("listView")} aria-pressed={viewMode === "list"} onClick={() => setViewMode("list")}><List size={16} /></button><button className={`icon-button ${viewMode === "thumbnails" ? "active" : ""}`} aria-label={t("thumbnailView")} title={t("thumbnailView")} aria-pressed={viewMode === "thumbnails"} onClick={() => setViewMode("thumbnails")}><LayoutGrid size={16} /></button></div><button className="toolbar-button" onClick={() => setNewFolderOpen(true)}><FolderPlus size={16} /><span>{t("newFolder")}</span></button><span className="toolbar-divider" /><button className="icon-button" aria-label={t("sortByName")} title={t("sortByName")}><ChevronDown size={16} /></button><button className="icon-button" aria-label={t("settings")} title={t("settings")} onClick={() => setSettingsOpen(true)}><MoreHorizontal size={17} /></button></div>
        <div className="file-table-wrap">{transfer && <div className="transfer-bar"><ClipboardPaste size={15} /><span>{t(transfer.mode === "copy" ? "copyPending" : "movePending", { name: transfer.entry.name })}</span><button onClick={() => void pasteInto(currentPath)}><ClipboardPaste size={14} /> {t("pasteHere")}</button><button className="icon-button" aria-label={t("cancel")} title={t("cancel")} onClick={() => setTransfer(null)}><X size={14} /></button></div>}{currentPath && <button className="back-row" onClick={() => void navigateTo(parentPath(currentPath))}><ArrowLeft size={14} /> {t("back")}</button>}
          {viewMode === "list" && <div className="table-header"><span>{t("name")}</span><span>{t("size")}</span><span>{t("modified")}</span><span /></div>}
          {loadingDirectory ? <div className="empty-state"><HardDrive size={20} /><span>{t("loadingDirectory")}</span></div> : visibleEntries.length ? viewMode === "list" ? visibleEntries.map((entry, index) => <button className={`file-row ${selectedPath === entry.path ? "selected" : ""}`} key={entry.path} onClick={() => openEntry(entry)} onDoubleClick={() => { if (entry.kind === "directory") openEntry(entry); else { setSelectedPath(entry.path); setPreviewEntry(entry); } }} onContextMenu={(event) => openContextMenu(event, entry)} style={{ "--row-index": index } as CSSProperties}>
            <span className="file-name-cell"><EntryIcon kind={entry.kind} /><span className="file-name-copy"><strong>{entry.name}</strong>{entry.kind === "directory" && <small>{t("directory")}</small>}</span>{entry.kind === "directory" && <ChevronRight className="row-chevron" size={15} />}</span><span className="file-meta">{entry.kind === "directory" ? "—" : formatBytes(entry.sizeBytes)}</span><span className="file-meta modified-meta">{formatModified(entry.modifiedMs, locale)}</span><span className="row-menu" onClick={(event) => { event.stopPropagation(); openContextMenu(event, entry); }}><MoreHorizontal size={16} /></span>
          </button>) : <div className="thumbnail-grid">{visibleEntries.map((entry, index) => { const isRemote = directoryConfigs.some((directory) => directory.path === activeDirectory && directory.kind && directory.kind !== "local"); return <button className={`thumbnail-card ${selectedPath === entry.path ? "selected" : ""}`} key={entry.path} onClick={() => openEntry(entry)} onDoubleClick={() => { if (entry.kind === "directory") openEntry(entry); else { setSelectedPath(entry.path); setPreviewEntry(entry); } }} onContextMenu={(event) => openContextMenu(event, entry)} style={{ "--row-index": index } as CSSProperties}><span className="thumbnail-image">{entry.kind === "image" ? <><EntryIcon kind={entry.kind} size={30} />{isRemote ? <RemoteThumbnail path={entry.path} /> : <img src={getThumbnailUrl(entry)} alt="" loading="lazy" onError={(event) => { event.currentTarget.style.display = "none"; }} />}</> : <EntryIcon kind={entry.kind} size={34} />}</span><span className="thumbnail-name" title={entry.name}>{entry.name}</span><span className="thumbnail-meta">{entry.kind === "directory" ? t("directory") : formatBytes(entry.sizeBytes)}</span></button>; })}</div> : <div className="empty-state"><Search size={20} /><strong>{t("noMatches")}</strong><span>{t("tryKeywords")}</span></div>}
          {rootPath && <div className="table-footnote"><span className="footnote-dot" />{rootPath}{currentPath ? `/${currentPath}` : ""}</div>}
        </div>
        <footer className="statusbar"><span><span className="status-indicator" />{rootPath ? t("localDirectoryActive") : t("chooseFolder")}</span><span>R2 URL <strong>{rules.baseUrl.replace(/^https?:\/\//, "")}</strong></span></footer>
      </section>

      <aside className="inspector"><div className="inspector-header"><div><span className="section-label">{t("quickLook")}</span><h2>{t("preview")}</h2></div><button className="icon-button" aria-label={t("morePreviewActions")} title={t("morePreviewActions")} onClick={() => selectedEntry && openContextMenuForButton(selectedEntry)}><MoreHorizontal size={18} /></button></div>
        {selectedEntry ? <><div className={`preview-stage ${selectedEntry.kind !== "image" ? "non-image-stage" : ""}`} onContextMenu={(event) => openPreviewContextMenu(event, selectedEntry)}><PreviewContent entry={selectedEntry} bytes={previewBytes} objectUrl={previewUrl} loading={previewLoading} error={previewError} t={t} /><button className="preview-expand" aria-label={t("openFile")} title={t("openFile")} onClick={() => setPreviewEntry(selectedEntry)}><ArrowDownToLine size={15} /></button></div>
          <div className="preview-caption"><span className="filetype-label">{selectedEntry.extension.toUpperCase() || "FILE"}</span><span className="file-size-label">{formatBytes(selectedEntry.sizeBytes)}</span></div><h3 className="preview-title">{selectedEntry.name}</h3><div className="preview-path"><Folder size={13} /> {parentPath(selectedEntry.path) || rootPath}</div>
          <div className="inspector-divider" /><div className="inspector-section-title"><span>{t("r2ObjectUrl")}</span></div><div className="url-preview"><span>{buildObjectUrl(selectedEntry.path, rules)}</span></div>
          <div className="inline-copy-actions">
            <button className="copy-link-button" onClick={() => copyLinks(selectedEntry, "url")}><Copy size={15} />{t("copyUrl")}</button>
            <button className="copy-link-button secondary" onClick={() => copyLinks(selectedEntry, "markdown")}><FileText size={15} />{t("markdown")}</button>
            <button className="copy-link-button secondary" onClick={() => copyLinks(selectedEntry, "html")}><Archive size={15} />{t("htmlLinks")}</button>
          </div>
          <div className="details-list"><div><span>{t("addedAt")}</span><strong>{new Intl.DateTimeFormat(locales.find((item) => item.id === locale)?.htmlLang, { dateStyle: "medium", timeStyle: "short" }).format(new Date())}</strong></div><div><span>{t("storageLocation")}</span><strong>{t("localStorageLabel")}</strong></div><div><span>{t("publicStatus")}</span><strong className="public-state"><span />{t("ruleConfigured")}</strong></div></div><button className="share-options" onClick={() => openContextMenuForButton(selectedEntry)}><span><Image size={15} />{t("copyOtherFormats")}</span><ChevronRight size={15} /></button></>
          : <div className="inspector-empty"><Image size={24} /><span>{t("chooseFileToPreview")}</span></div>}
        <div className="inspector-footer"><span><ShieldCheck size={14} /> {t("noR2Sync")}</span></div>
      </aside>

      {previewEntry && <div className="file-preview-backdrop" onMouseDown={(event) => event.target === event.currentTarget && setPreviewEntry(null)}><section className="file-preview-modal" role="dialog" aria-modal="true" aria-label={`${t("preview")}: ${previewEntry.name}`} onContextMenu={(event) => openPreviewContextMenu(event, previewEntry)}><header className="file-preview-header"><div className="file-preview-heading"><span className="section-label">{t("preview")}</span><h2>{previewEntry.name}</h2></div><button className="icon-button" aria-label={t("close")} title={t("close")} onClick={() => setPreviewEntry(null)}><X size={18} /></button></header><div className="file-preview-content"><PreviewContent entry={previewEntry} bytes={previewBytes} objectUrl={previewUrl} loading={previewLoading} error={previewError} t={t} /></div><footer className="file-preview-footer"><span>{previewEntry.extension.toUpperCase() || "FILE"}</span><span>{formatBytes(previewEntry.sizeBytes)}</span></footer></section></div>}
      {menu && <div className="context-menu" style={{ left: menu.x, top: menu.y }} onClick={(event) => event.stopPropagation()}><div className="context-title">{menu.preview ? t("copyUrl") : t("copyLinks")} <span>{menu.entry.kind === "directory" ? t("directory") : t("file")}</span></div><button onClick={() => copyLinks(menu.entry, "url")}><Copy size={15} /><span>{t("plainTextUrl")}</span><kbd>TXT</kbd></button><button onClick={() => copyLinks(menu.entry, "markdown")}><FileText size={15} /><span>{t("markdown")}</span><kbd>MD</kbd></button><button onClick={() => copyLinks(menu.entry, "html")}><Archive size={15} /><span>{t("htmlLinks")}</span><kbd>HTML</kbd></button>{menu.entry.kind === "directory" && !menu.preview && <div className="context-hint">{t("recursiveCopyHint")}</div>}{!menu.preview && <><div className="context-divider" /><button onClick={() => startTransfer(menu.entry, "copy")}><Copy size={15} /><span>{t("copy")}</span></button><button onClick={() => startTransfer(menu.entry, "move")}><Move size={15} /><span>{t("move")}</span></button><button className="danger" onClick={() => { setDeleteTarget(menu.entry); setMenu(null); }}><Trash2 size={15} /><span>{t("delete")}</span></button>{transfer && <button onClick={() => void pasteInto(menu.entry.kind === "directory" ? menu.entry.path : currentPath)}><ClipboardPaste size={15} /><span>{t("pasteHere")}</span></button>}</>}{menu.preview && <div className="context-hint">{t("previewContextHint")}</div>}</div>}
      {networkOpen && <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && setNetworkOpen(false)}><form className="settings-modal network-modal" onSubmit={connectRemote}><div className="modal-heading"><div><span className="section-label">{t("locations")}</span><h2>{t("connectWebdav")}</h2></div><button type="button" className="icon-button" aria-label={t("close")} onClick={() => setNetworkOpen(false)}><X size={18} /></button></div><label className="field-label">{t("serverAddress")}<input autoFocus required value={networkDraft.endpoint} onChange={(event) => setNetworkDraft({ ...networkDraft, endpoint: event.currentTarget.value })} placeholder="https://dav.example.com/remote.php/dav/files/user" /></label><label className="field-label">{t("username")}<input autoComplete="username" value={networkDraft.username} onChange={(event) => setNetworkDraft({ ...networkDraft, username: event.currentTarget.value })} /></label><label className="field-label">{t("password")}<input type="password" autoComplete="current-password" value={networkDraft.password} onChange={(event) => setNetworkDraft({ ...networkDraft, password: event.currentTarget.value })} /></label><div className="modal-actions"><button type="button" className="button-secondary" onClick={() => setNetworkOpen(false)}>{t("cancel")}</button><button className="button-primary" type="submit" disabled={loadingDirectory}><Cloud size={15} />{t("connect")}</button></div></form></div>}
      {settingsOpen && <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && setSettingsOpen(false)}><section className="settings-modal settings-page" role="dialog" aria-modal="true" aria-label={t("settings")}>
        <div className="modal-heading"><div><span className="section-label">R2 LIST</span><h2>{t("settings")}</h2></div><button type="button" className="icon-button" aria-label={t("close")} onClick={() => setSettingsOpen(false)}><X size={18} /></button></div>
        <p className="modal-description">{t("settingsDescription")}</p>
        <div className="settings-section">
          <h3>{t("configuration")}</h3>
          <div className="settings-row"><span>{t("configDirectory")}</span><code title={configLocation?.directory}>{configLocation?.directory ?? "—"}</code></div>
          <div className="settings-row"><span>{t("configFilePath")}</span><code title={configLocation?.configPath}>{configLocation?.configPath ?? "—"}</code></div>
          <p className="settings-hint">{t("configDirectoryHint")}</p>
          <div className="settings-actions"><button type="button" className="button-secondary" onClick={() => void chooseConfigDirectory()} disabled={configSaving}><Folder size={15} />{t("chooseFolder")}</button><button type="button" className="button-secondary" onClick={() => void resetConfigDirectory()} disabled={configSaving}>{t("useDefaultDirectory")}</button></div>
        </div>
        <div className="settings-section">
          <h3>{t("versionCheck")}</h3>
          <div className="settings-row"><span>{t("version")}</span><strong>{appVersion || "0.0.3"}</strong></div>
          <div className="settings-row"><span>{t("latestVersion")}</span><span className={`update-status-pill ${updateStatus.status}`}>{updateStatus.message || t("checkingUpdates")}{updateStatus.latest ? <small> · {updateStatus.latest}</small> : null}</span></div>
          <div className="settings-actions"><button type="button" className="button-secondary" onClick={() => setVersionCheckId((value) => value + 1)}><RefreshCw size={15} />{t("checkForUpdates")}</button><button type="button" className="button-secondary" onClick={() => void openUrl(GITHUB_RELEASES_URL)}>{t("viewLatestRelease")}</button></div>
        </div>
        <div className="settings-section">
          <h3>{t("githubConnection")}</h3>
          <p className="settings-hint">{t("githubDescription")}</p>
          <div className="settings-row"><span>{t("githubConnection")}</span><span className={`connection-pill ${githubStatus}`}>{githubStatus === "checking" ? t("checkingUpdates") : githubStatus === "ok" ? t("connectionOk") : githubStatus === "error" ? t("connectionFailed") : "—"}</span></div>
          <div className="settings-actions"><button type="button" className="button-secondary" onClick={openGithubProject}><Globe size={15} />{t("openRepository")}</button><button type="button" className="button-secondary" onClick={() => void testGithubConnection()} disabled={githubStatus === "checking"}><RefreshCw size={15} />{t("connect")}</button></div>
        </div>
        <div className="modal-actions"><button type="button" className="button-primary" onClick={() => setSettingsOpen(false)}>{t("close")}</button></div>
      </section></div>}
      {editingDirectory && <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && setEditingDirectory(null)}><form className="settings-modal settings-page" onSubmit={saveLocation}>
        <div className="modal-heading"><div><span className="section-label">{t("locations")}</span><h2>{t("editLocation")}</h2></div><button type="button" className="icon-button" aria-label={t("close")} onClick={() => setEditingDirectory(null)}><X size={18} /></button></div>
        <label className="field-label">{t("locationLabel")}<input value={locationDraft.label} onChange={(event) => setLocationDraft({ ...locationDraft, label: event.currentTarget.value })} placeholder={editingDirectory.split(/[\\/]/).filter(Boolean).pop()} /></label>
        {editingConfig?.kind === "webdav" && <><label className="field-label">{t("serverAddress")}<input value={locationDraft.endpoint} onChange={(event) => setLocationDraft({ ...locationDraft, endpoint: event.currentTarget.value })} placeholder="https://dav.example.com/remote.php/dav/files/user" /></label><label className="field-label">{t("username")}<input autoComplete="username" value={locationDraft.username} onChange={(event) => setLocationDraft({ ...locationDraft, username: event.currentTarget.value })} /></label><label className="field-label">{t("password")}<input type="password" autoComplete="current-password" value={locationDraft.password} onChange={(event) => setLocationDraft({ ...locationDraft, password: event.currentTarget.value })} /></label></>}
        <h3 className="settings-section-title">{t("locationRules")}</h3>
        <p className="modal-description">{t("rulesDescription")}</p>
        <label className="field-label">{t("publicBase")}<input value={locationDraft.rules.baseUrl} onChange={(event) => setLocationDraft({ ...locationDraft, rules: { ...locationDraft.rules, baseUrl: event.currentTarget.value } })} placeholder="https://cdn.example.com" /></label>
        <label className="field-label">{t("objectPrefix")}<input value={locationDraft.rules.prefix} onChange={(event) => setLocationDraft({ ...locationDraft, rules: { ...locationDraft.rules, prefix: event.currentTarget.value } })} placeholder={t("optional")} /></label>
        <label className="field-label">{t("bucketName")} <span>{t("bucketReferenced")}</span><input value={locationDraft.rules.bucket ?? ""} onChange={(event) => setLocationDraft({ ...locationDraft, rules: { ...locationDraft.rules, bucket: event.currentTarget.value } })} placeholder={t("optional")} /></label>
        <label className="field-label">{t("advancedTemplate")} <span>{t("optional")}</span><input value={locationDraft.rules.template ?? ""} onChange={(event) => setLocationDraft({ ...locationDraft, rules: { ...locationDraft.rules, template: event.currentTarget.value } })} placeholder="{base}/{key}" /></label>
        <div className="template-help"><span>{"{base}"}</span>{t("helpBase")} <span>{"{key}"}</span>{t("helpKey")} <span>{"{bucket}"}</span>{t("helpBucket")}</div><div className="modal-preview"><span>{t("previewSample")}</span><code>{safePreview(selectedEntry?.path ?? "folder/file.jpg")}</code></div>
        <div className="modal-actions"><button type="button" className="button-secondary" onClick={() => setEditingDirectory(null)}>{t("cancel")}</button><button className="button-primary" type="submit"><Check size={15} />{t("saveLocation")}</button></div>
      </form></div>}
      {deleteTarget && <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && setDeleteTarget(null)}><section className="settings-modal" role="dialog" aria-modal="true" aria-label={t("deleteConfirmTitle")}><div className="modal-heading"><div><span className="section-label">{t("deleteConfirmTitle")}</span><h2>{deleteTarget.name}</h2></div><button type="button" className="icon-button" aria-label={t("close")} onClick={() => setDeleteTarget(null)}><X size={18} /></button></div><p className="modal-description">{t("deleteConfirm", { name: deleteTarget.name })}</p><div className="modal-actions"><button type="button" className="button-secondary" onClick={() => setDeleteTarget(null)}>{t("cancel")}</button><button type="button" className="button-primary danger-button" onClick={() => void confirmDelete()}><Trash2 size={15} />{t("delete")}</button></div></section></div>}
      {newFolderOpen && <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && setNewFolderOpen(false)}><form className="settings-modal" onSubmit={createFolder}><div className="modal-heading"><div><span className="section-label">{t("localDirectoryActive")}</span><h2>{t("newFolder")}</h2></div><button type="button" className="icon-button" aria-label={t("close")} onClick={() => setNewFolderOpen(false)}><X size={18} /></button></div><label className="field-label">{t("folderName")}<input autoFocus value={newFolderName} onChange={(event) => setNewFolderName(event.currentTarget.value)} required maxLength={255} /></label><div className="modal-actions"><button type="button" className="button-secondary" onClick={() => setNewFolderOpen(false)}>{t("cancel")}</button><button className="button-primary" type="submit"><FolderPlus size={15} />{t("createFolder")}</button></div></form></div>}
      {toast && <div className="toast"><Check size={15} />{toast}</div>}
    </main>
  );
}

export default App;
