import { useEffect, useState, type CSSProperties, type FormEvent, type MouseEvent } from "react";
import {
  Archive, ArrowDownToLine, ArrowLeft, Check, ChevronDown, ChevronRight, Cloud, Copy,
  File, FileImage, FileText, Film, Folder, FolderPlus, HardDrive, Image, MoreHorizontal,
  Languages, Moon, Plus, Search, Settings2, ShieldCheck, Sun, Upload, X,
} from "lucide-react";
import { buildObjectUrl, formatLinks, type ShareFormat, type UrlRules } from "./lib/urlMapping";
import { detectLocale, formatItemCount, isLocale, locales, translate, type Locale, type TranslationKey } from "./i18n";
import { resolveTheme, type ThemePreference } from "./theme";
import "./App.css";
import "./preferences.css";

type EntryKind = "directory" | "image" | "video" | "document" | "archive" | "file";
type Entry = { path: string; name: string; kind: EntryKind; size: string; modified: string; previewUrl?: string };
type MenuState = { x: number; y: number; entry: Entry } | null;

const entries: Entry[] = [
  { path: "Campaign 2026", name: "Campaign 2026", kind: "directory", size: "—", modified: "Today" },
  { path: "Brand Library", name: "Brand Library", kind: "directory", size: "—", modified: "Yesterday" },
  { path: "Blue hour over the coast.jpg", name: "Blue hour over the coast.jpg", kind: "image", size: "8.4 MB", modified: "Today, 10:42", previewUrl: "https://images.unsplash.com/photo-1470770841072-f978cf4d019e?auto=format&fit=crop&w=1000&q=85" },
  { path: "Launch notes.md", name: "Launch notes.md", kind: "document", size: "12 KB", modified: "Today, 09:18" },
  { path: "Q3 product overview.pdf", name: "Q3 product overview.pdf", kind: "document", size: "2.1 MB", modified: "Oct 09, 2026" },
  { path: "Campaign 2026/launch-film.mp4", name: "launch-film.mp4", kind: "video", size: "184 MB", modified: "Today, 10:16" },
  { path: "Campaign 2026/press-kit.pdf", name: "press-kit.pdf", kind: "document", size: "4.8 MB", modified: "Yesterday" },
  { path: "Campaign 2026/social-cut-vertical.mp4", name: "social-cut-vertical.mp4", kind: "video", size: "62 MB", modified: "Yesterday" },
  { path: "Brand Library/brand-mark.svg", name: "brand-mark.svg", kind: "image", size: "48 KB", modified: "Oct 08, 2026" },
  { path: "Brand Library/type-specimen.pdf", name: "type-specimen.pdf", kind: "document", size: "1.3 MB", modified: "Oct 07, 2026" },
  { path: "Brand Library/source-files.zip", name: "source-files.zip", kind: "archive", size: "246 MB", modified: "Oct 06, 2026" },
];

const initialRules: UrlRules = { baseUrl: "https://cdn.studio-example.com", prefix: "northstar/media", template: "" };
const parentPath = (path: string) => path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";

function loadRules(): UrlRules {
  try {
    const saved = localStorage.getItem("r2-list-url-rules");
    return saved ? { ...initialRules, ...JSON.parse(saved) as Partial<UrlRules> } : initialRules;
  } catch {
    return initialRules;
  }
}

function loadLocale(): Locale {
  const saved = localStorage.getItem("r2-list-language");
  return isLocale(saved) ? saved : detectLocale(navigator.language);
}

function loadThemePreference(): ThemePreference {
  const saved = localStorage.getItem("r2-list-theme-mode");
  if (saved === "system" || saved === "light" || saved === "dark") return saved;
  return "system";
}

function formatModified(value: string, locale: Locale): string {
  const t = (key: TranslationKey) => translate(locale, key);
  if (value === "Today") return t("today");
  if (value.startsWith("Today, ")) return `${t("today")} · ${value.slice(7)}`;
  if (value === "Yesterday") return t("yesterday");
  if (value.startsWith("Yesterday, ")) return `${t("yesterday")} · ${value.slice(11)}`;

  const date = value.match(/^Oct (\d+), (\d{4})$/);
  if (date) {
    return new Intl.DateTimeFormat(locales.find((item) => item.id === locale)?.htmlLang, {
      dateStyle: "medium",
    }).format(new Date(Number(date[2]), 9, Number(date[1])));
  }
  return value;
}

function EntryIcon({ kind, size = 18 }: { kind: EntryKind; size?: number }) {
  if (kind === "directory") return <Folder size={size} className="entry-icon folder-icon" />;
  if (kind === "image") return <FileImage size={size} className="entry-icon image-icon" />;
  if (kind === "video") return <Film size={size} className="entry-icon video-icon" />;
  if (kind === "document") return <FileText size={size} className="entry-icon document-icon" />;
  if (kind === "archive") return <Archive size={size} className="entry-icon archive-icon" />;
  return <File size={size} className="entry-icon" />;
}

function App() {
  const [currentPath, setCurrentPath] = useState("");
  const [selectedPath, setSelectedPath] = useState(entries[2].path);
  const [search, setSearch] = useState("");
  const [rules, setRules] = useState<UrlRules>(loadRules);
  const [draftRules, setDraftRules] = useState<UrlRules>(loadRules);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [locale, setLocale] = useState<Locale>(loadLocale);
  const [themePreference, setThemePreference] = useState<ThemePreference>(loadThemePreference);
  const [systemDark, setSystemDark] = useState(() => window.matchMedia("(prefers-color-scheme: dark)").matches);
  const [menu, setMenu] = useState<MenuState>(null);
  const [toast, setToast] = useState("");
  const theme = resolveTheme(themePreference, systemDark);
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
    if (!toast) return;
    const timeout = window.setTimeout(() => setToast(""), 2600);
    return () => window.clearTimeout(timeout);
  }, [toast]);

  const visibleEntries = entries
    .filter((entry) => parentPath(entry.path) === currentPath)
    .filter((entry) => entry.name.toLowerCase().includes(search.toLowerCase()))
    .sort((a, b) => Number(b.kind === "directory") - Number(a.kind === "directory") || a.name.localeCompare(b.name));
  const selectedEntry = entries.find((entry) => entry.path === selectedPath) ?? visibleEntries[0];
  const pathParts = currentPath ? currentPath.split("/") : [];

  function showNotConnected() { setToast(t("realDirectoryPending")); }
  function openRules() { setDraftRules(rules); setSettingsOpen(true); }
  function openEntry(entry: Entry) {
    if (entry.kind === "directory") { setCurrentPath(entry.path); setSearch(""); setSelectedPath(""); }
    else setSelectedPath(entry.path);
  }
  function openContextMenu(event: MouseEvent, entry: Entry) {
    event.preventDefault();
    setSelectedPath(entry.path);
    setMenu({ x: Math.min(event.clientX, window.innerWidth - 224), y: Math.min(event.clientY, window.innerHeight - 220), entry });
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
  function saveRules(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      buildObjectUrl("sample-check.txt", draftRules);
      setRules(draftRules);
      localStorage.setItem("r2-list-url-rules", JSON.stringify(draftRules));
      setSettingsOpen(false);
      setToast(t("rulesSaved"));
    } catch { setToast(t("invalidRules")); }
  }
  function safePreview(path: string) {
    try { return buildObjectUrl(path, draftRules); }
    catch { return t("invalidRules"); }
  }

  return (
    <main className="app-shell" onClick={() => menu && setMenu(null)}>
      <aside className="sidebar">
        <div className="brand-lockup"><div className="brand-mark"><Cloud size={19} strokeWidth={2.3} /></div><div><strong>R2 List</strong><span>{t("objectWorkspace")}</span></div><button className="icon-button sidebar-more" aria-label={t("viewOptions")} title={t("viewOptions")}><MoreHorizontal size={18} /></button></div>
        <div className="sidebar-section"><div className="section-label">{t("workspace")}</div><button className="nav-item active"><HardDrive size={17} /><span>{t("files")}</span><span className="nav-count">{entries.length}</span></button><button className="nav-item" onClick={openRules}><Settings2 size={17} /><span>{t("urlRules")}</span></button></div>
        <div className="sidebar-section locations-section"><div className="section-heading"><span className="section-label">{t("locations")}</span><button className="icon-button tiny" aria-label={t("addLocation")} title={t("addLocation")} onClick={showNotConnected}><Plus size={15} /></button></div>
          <button className="location-item selected"><span className="location-dot local-dot" /><span className="location-copy"><strong>{t("mediaLibrary")}</strong><small>{t("localSample")}</small></span><ChevronRight size={14} /></button>
          <button className="location-item" onClick={showNotConnected}><span className="location-dot remote-dot" /><span className="location-copy"><strong>{t("studioNas")}</strong><small>{t("smbOffline")}</small></span><span className="offline-dot" /></button>
          <button className="add-location" onClick={showNotConnected}><Plus size={14} /> {t("addLocation")}</button>
        </div>
        <div className="sidebar-bottom"><div className="sync-note"><span className="note-icon"><ShieldCheck size={15} /></span><span><strong>{t("urlOnly")}</strong><small>{t("externalSync")}</small></span></div>
          <div className="profile-button"><span className="avatar">R</span><span className="profile-copy"><strong>R2 List</strong><small>{t("personalWorkspace")}</small></span></div>
        </div>
      </aside>

      <section className="content-area">
        <header className="topbar"><div className="breadcrumbs"><button className="breadcrumb-root" onClick={() => { setCurrentPath(""); setSearch(""); }}>{t("mediaLibrary")}</button>{pathParts.map((part, index) => { const targetPath = pathParts.slice(0, index + 1).join("/"); return <span className="breadcrumb-part" key={targetPath}><ChevronRight size={13} /><button onClick={() => setCurrentPath(targetPath)}>{part}</button></span>; })}</div>
          <div className="topbar-actions"><span className="connection-state"><span />{t("local")}</span><label className="preference-select" title={t("language")}><Languages size={14} /><select aria-label={t("language")} value={locale} onChange={(event) => { if (isLocale(event.currentTarget.value)) setLocale(event.currentTarget.value); }}>{locales.map((item) => <option value={item.id} key={item.id}>{item.label}</option>)}</select><ChevronDown size={12} /></label><label className="preference-select theme-select" title={t("theme")}>{theme === "dark" ? <Moon size={14} /> : <Sun size={14} />}<select aria-label={t("theme")} value={themePreference} onChange={(event) => setThemePreference(event.currentTarget.value as ThemePreference)}><option value="system">{t("themeSystem")}</option><option value="light">{t("themeLight")}</option><option value="dark">{t("themeDark")}</option></select><ChevronDown size={12} /></label><button className="icon-button" aria-label={t("urlRules")} title={t("urlRules")} onClick={openRules}><Settings2 size={17} /></button></div>
        </header>
        <div className="workspace-heading"><div><div className="eyebrow"><span className="eyebrow-line" />{t("localDirectory")} <span className="muted-separator">/</span> {t("urlMapped")}</div><h1>{pathParts[pathParts.length - 1] ?? t("mediaLibrary")}</h1><p className="workspace-path"><HardDrive size={13} /> {t("samplePath")} <span className="path-dot">·</span> {formatItemCount(locale, visibleEntries.length)}</p></div>
          <div className="heading-actions"><button className="button-secondary" onClick={openRules}><Settings2 size={15} /> {t("urlRules")}</button><button className="button-primary" onClick={showNotConnected}><Upload size={15} /> {t("addFiles")}</button></div>
        </div>
        <div className="toolbar"><div className="search-box"><Search size={16} /><input aria-label={t("searchAria")} value={search} onChange={(event) => setSearch(event.currentTarget.value)} placeholder={t("searchPlaceholder")} /><kbd>⌘ F</kbd></div><div className="toolbar-spacer" /><button className="toolbar-button" onClick={showNotConnected}><FolderPlus size={16} /><span>{t("newFolder")}</span></button><span className="toolbar-divider" /><button className="icon-button" aria-label={t("sortByName")} title={t("sortByName")}><ChevronDown size={16} /></button><button className="icon-button" aria-label={t("viewOptions")} title={t("viewOptions")}><MoreHorizontal size={17} /></button></div>
        <div className="file-table-wrap">{currentPath && <button className="back-row" onClick={() => setCurrentPath(parentPath(currentPath))}><ArrowLeft size={14} /> {t("back")}</button>}
          <div className="table-header"><span>{t("name")}</span><span>{t("size")}</span><span>{t("modified")}</span><span /></div>
          {visibleEntries.length ? visibleEntries.map((entry, index) => <button className={`file-row ${selectedPath === entry.path ? "selected" : ""}`} key={entry.path} onClick={() => openEntry(entry)} onDoubleClick={() => entry.kind === "directory" && openEntry(entry)} onContextMenu={(event) => openContextMenu(event, entry)} style={{ "--row-index": index } as CSSProperties}>
            <span className="file-name-cell"><EntryIcon kind={entry.kind} /><span className="file-name-copy"><strong>{entry.name}</strong>{entry.kind === "directory" && <small>{t("directory")}</small>}</span>{entry.kind === "directory" && <ChevronRight className="row-chevron" size={15} />}</span><span className="file-meta">{entry.size}</span><span className="file-meta modified-meta">{formatModified(entry.modified, locale)}</span><span className="row-menu" onClick={(event) => { event.stopPropagation(); openContextMenu(event, entry); }}><MoreHorizontal size={16} /></span>
          </button>) : <div className="empty-state"><Search size={20} /><strong>{t("noMatches")}</strong><span>{t("tryKeywords")}</span></div>}
          <div className="table-footnote"><span className="footnote-dot" />{t("sampleDirectory")} <span>·</span> {t("realFilesNotConnected")}</div>
        </div>
        <footer className="statusbar"><span><span className="status-indicator" />{t("sampleMode")} · {t("fileOpsPending")}</span><span>R2 URL <strong>{rules.baseUrl.replace(/^https?:\/\//, "")}</strong></span></footer>
      </section>

      <aside className="inspector"><div className="inspector-header"><div><span className="section-label">{t("quickLook")}</span><h2>{t("preview")}</h2></div><button className="icon-button" aria-label={t("morePreviewActions")} title={t("morePreviewActions")} onClick={() => selectedEntry && openContextMenuForButton(selectedEntry)}><MoreHorizontal size={18} /></button></div>
        {selectedEntry ? <><div className={`preview-stage ${selectedEntry.kind !== "image" ? "non-image-stage" : ""}`}>{selectedEntry.kind === "image" && selectedEntry.previewUrl ? <img src={selectedEntry.previewUrl} alt={selectedEntry.name} /> : <div className={`preview-document preview-${selectedEntry.kind}`}><EntryIcon kind={selectedEntry.kind} size={34} /><span>{selectedEntry.name.split(".").pop()?.toUpperCase()}</span><small>{selectedEntry.kind === "video" ? t("videoPreviewPending") : t("previewPending")}</small></div>}<button className="preview-expand" aria-label={t("openFile")} title={t("openFile")}><ArrowDownToLine size={15} /></button></div>
          <div className="preview-caption"><span className="filetype-label">{selectedEntry.name.split(".").pop()?.toUpperCase() ?? "FILE"}</span><span className="file-size-label">{selectedEntry.size}</span></div><h3 className="preview-title">{selectedEntry.name}</h3><div className="preview-path"><Folder size={13} /> {parentPath(selectedEntry.path) || t("mediaLibrary")}</div>
          <div className="inspector-divider" /><div className="inspector-section-title"><span>{t("r2ObjectUrl")}</span><button className="text-action" onClick={openRules}>{t("editRules")}</button></div><div className="url-preview"><span>{buildObjectUrl(selectedEntry.path, rules)}</span></div><button className="copy-link-button" onClick={() => copyLinks(selectedEntry, "url")}><Copy size={15} />{t("copyUrl")}<ChevronDown size={14} /></button>
          <div className="details-list"><div><span>{t("addedAt")}</span><strong>{new Intl.DateTimeFormat(locales.find((item) => item.id === locale)?.htmlLang, { dateStyle: "medium", timeStyle: "short" }).format(new Date())}</strong></div><div><span>{t("storageLocation")}</span><strong>{t("localStorageLabel")}</strong></div><div><span>{t("publicStatus")}</span><strong className="public-state"><span />{t("ruleConfigured")}</strong></div></div><button className="share-options" onClick={() => openContextMenuForButton(selectedEntry)}><span><Image size={15} />{t("copyOtherFormats")}</span><ChevronRight size={15} /></button></>
          : <div className="inspector-empty"><Image size={24} /><span>{t("chooseFileToPreview")}</span></div>}
        <div className="inspector-footer"><span><ShieldCheck size={14} /> {t("noR2Sync")}</span></div>
      </aside>

      {menu && <div className="context-menu" style={{ left: menu.x, top: menu.y }} onClick={(event) => event.stopPropagation()}><div className="context-title">{t("copyLinks")} <span>{menu.entry.kind === "directory" ? t("directory") : t("file")}</span></div><button onClick={() => copyLinks(menu.entry, "url")}><Copy size={15} /><span>{t("plainTextUrl")}</span><kbd>TXT</kbd></button><button onClick={() => copyLinks(menu.entry, "markdown")}><FileText size={15} /><span>{t("markdown")}</span><kbd>MD</kbd></button><button onClick={() => copyLinks(menu.entry, "html")}><Archive size={15} /><span>{t("htmlLinks")}</span><kbd>HTML</kbd></button>{menu.entry.kind === "directory" && <div className="context-hint">{t("recursiveCopyHint")}</div>}</div>}
      {settingsOpen && <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && setSettingsOpen(false)}><form className="settings-modal" onSubmit={saveRules}>
        <div className="modal-heading"><div><span className="section-label">{t("linkConfiguration")}</span><h2>{t("rulesTitle")}</h2></div><button type="button" className="icon-button" aria-label={t("close")} onClick={() => setSettingsOpen(false)}><X size={18} /></button></div>
        <p className="modal-description">{t("rulesDescription")}</p>
        <label className="field-label">{t("publicBase")}<input value={draftRules.baseUrl} onChange={(event) => setDraftRules({ ...draftRules, baseUrl: event.currentTarget.value })} placeholder="https://cdn.example.com" /></label>
        <label className="field-label">{t("objectPrefix")}<input value={draftRules.prefix} onChange={(event) => setDraftRules({ ...draftRules, prefix: event.currentTarget.value })} placeholder={t("optional")} /></label>
        <label className="field-label">{t("bucketName")} <span>{t("bucketReferenced")}</span><input value={draftRules.bucket ?? ""} onChange={(event) => setDraftRules({ ...draftRules, bucket: event.currentTarget.value })} placeholder={t("optional")} /></label>
        <label className="field-label">{t("advancedTemplate")} <span>{t("optional")}</span><input value={draftRules.template ?? ""} onChange={(event) => setDraftRules({ ...draftRules, template: event.currentTarget.value })} placeholder="{base}/{key}" /></label>
        <div className="template-help"><span>{"{base}"}</span>{t("helpBase")} <span>{"{key}"}</span>{t("helpKey")} <span>{"{bucket}"}</span>{t("helpBucket")}</div><div className="modal-preview"><span>{t("previewSample")}</span><code>{safePreview(selectedEntry?.path ?? "folder/file.jpg")}</code></div>
        <div className="modal-actions"><button type="button" className="button-secondary" onClick={() => setSettingsOpen(false)}>{t("cancel")}</button><button className="button-primary" type="submit"><Check size={15} />{t("saveRules")}</button></div>
      </form></div>}
      {toast && <div className="toast"><Check size={15} />{toast}</div>}
    </main>
  );
}

export default App;
