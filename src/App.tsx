import { useEffect, useState, type CSSProperties, type FormEvent, type MouseEvent } from "react";
import {
  Archive, ArrowDownToLine, ArrowLeft, Check, ChevronDown, ChevronRight, Cloud, Copy,
  File, FileImage, FileText, Film, Folder, FolderPlus, HardDrive, Image, MoreHorizontal,
  Moon, Plus, Search, Settings2, ShieldCheck, Sun, Upload, X,
} from "lucide-react";
import { buildObjectUrl, formatLinks, type ShareFormat, type UrlRules } from "./lib/urlMapping";
import "./App.css";

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
  const [theme, setTheme] = useState<"light" | "dark">(() => localStorage.getItem("r2-list-theme") === "dark" ? "dark" : "light");
  const [menu, setMenu] = useState<MenuState>(null);
  const [toast, setToast] = useState("");

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("r2-list-theme", theme);
  }, [theme]);
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

  function showNotConnected() { setToast("真实目录访问将在 Rust 文件系统后端接入后启用"); }
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
    if (!links.length) { setToast("此目录目前没有可复制链接的文件"); return; }
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
      setToast(`已复制 ${links.length} 个链接 · ${format.toUpperCase()}`);
    } catch { setToast("剪贴板不可用，请检查应用权限"); }
  }
  function saveRules(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      buildObjectUrl("sample-check.txt", draftRules);
      setRules(draftRules);
      localStorage.setItem("r2-list-url-rules", JSON.stringify(draftRules));
      setSettingsOpen(false);
      setToast("URL 规则已保存");
    } catch (error) { setToast(error instanceof Error ? error.message : "URL 规则无效"); }
  }
  function safePreview(path: string) {
    try { return buildObjectUrl(path, draftRules); }
    catch (error) { return error instanceof Error ? error.message : "无效规则"; }
  }

  return (
    <main className="app-shell" onClick={() => menu && setMenu(null)}>
      <aside className="sidebar">
        <div className="brand-lockup"><div className="brand-mark"><Cloud size={19} strokeWidth={2.3} /></div><div><strong>R2 List</strong><span>OBJECT WORKSPACE</span></div><button className="icon-button sidebar-more" aria-label="更多选项" title="更多选项"><MoreHorizontal size={18} /></button></div>
        <div className="sidebar-section"><div className="section-label">WORKSPACE</div><button className="nav-item active"><HardDrive size={17} /><span>文件</span><span className="nav-count">{entries.length}</span></button><button className="nav-item" onClick={openRules}><Settings2 size={17} /><span>URL 规则</span></button></div>
        <div className="sidebar-section locations-section"><div className="section-heading"><span className="section-label">LOCATIONS</span><button className="icon-button tiny" aria-label="添加位置" title="添加位置" onClick={showNotConnected}><Plus size={15} /></button></div>
          <button className="location-item selected"><span className="location-dot local-dot" /><span className="location-copy"><strong>Media Library</strong><small>本地目录 · 示例</small></span><ChevronRight size={14} /></button>
          <button className="location-item" onClick={showNotConnected}><span className="location-dot remote-dot" /><span className="location-copy"><strong>Studio NAS</strong><small>SMB · 未连接</small></span><span className="offline-dot" /></button>
          <button className="add-location" onClick={showNotConnected}><Plus size={14} /> 添加位置</button>
        </div>
        <div className="sidebar-bottom"><div className="sync-note"><span className="note-icon"><ShieldCheck size={15} /></span><span><strong>仅映射 URL</strong><small>同步由外部服务负责</small></span></div>
          <button className="profile-button" onClick={() => setTheme(theme === "light" ? "dark" : "light")}><span className="avatar">R</span><span className="profile-copy"><strong>R2 List</strong><small>个人工作区</small></span>{theme === "light" ? <Moon size={16} /> : <Sun size={16} />}</button>
        </div>
      </aside>

      <section className="content-area">
        <header className="topbar"><div className="breadcrumbs"><button className="breadcrumb-root" onClick={() => { setCurrentPath(""); setSearch(""); }}>Media Library</button>{pathParts.map((part, index) => { const targetPath = pathParts.slice(0, index + 1).join("/"); return <span className="breadcrumb-part" key={targetPath}><ChevronRight size={13} /><button onClick={() => setCurrentPath(targetPath)}>{part}</button></span>; })}</div>
          <div className="topbar-actions"><span className="connection-state"><span />LOCAL</span><button className="icon-button" aria-label="URL 规则设置" title="URL 规则设置" onClick={openRules}><Settings2 size={17} /></button><button className="avatar mini-avatar" aria-label="切换深色模式" title="切换深色模式" onClick={() => setTheme(theme === "light" ? "dark" : "light")}>{theme === "light" ? <Moon size={16} /> : <Sun size={16} />}</button></div>
        </header>
        <div className="workspace-heading"><div><div className="eyebrow"><span className="eyebrow-line" />LOCAL DIRECTORY <span className="muted-separator">/</span> R2 URL MAPPED</div><h1>{pathParts[pathParts.length - 1] ?? "Media Library"}</h1><p className="workspace-path"><HardDrive size={13} /> ~/Studio/Media Library <span className="path-dot">·</span> {visibleEntries.length} items</p></div>
          <div className="heading-actions"><button className="button-secondary" onClick={openRules}><Settings2 size={15} /> URL 规则</button><button className="button-primary" onClick={showNotConnected}><Upload size={15} /> 添加文件</button></div>
        </div>
        <div className="toolbar"><div className="search-box"><Search size={16} /><input aria-label="搜索文件" value={search} onChange={(event) => setSearch(event.currentTarget.value)} placeholder="搜索文件或文件夹…" /><kbd>⌘ F</kbd></div><div className="toolbar-spacer" /><button className="toolbar-button" onClick={showNotConnected}><FolderPlus size={16} /><span>新建文件夹</span></button><span className="toolbar-divider" /><button className="icon-button" aria-label="排序方式" title="按名称排序"><ChevronDown size={16} /></button><button className="icon-button" aria-label="更多视图选项" title="视图选项"><MoreHorizontal size={17} /></button></div>
        <div className="file-table-wrap">{currentPath && <button className="back-row" onClick={() => setCurrentPath(parentPath(currentPath))}><ArrowLeft size={14} /> 返回上级</button>}
          <div className="table-header"><span>名称</span><span>大小</span><span>修改时间</span><span /></div>
          {visibleEntries.length ? visibleEntries.map((entry, index) => <button className={`file-row ${selectedPath === entry.path ? "selected" : ""}`} key={entry.path} onClick={() => openEntry(entry)} onDoubleClick={() => entry.kind === "directory" && openEntry(entry)} onContextMenu={(event) => openContextMenu(event, entry)} style={{ "--row-index": index } as CSSProperties}>
            <span className="file-name-cell"><EntryIcon kind={entry.kind} /><span className="file-name-copy"><strong>{entry.name}</strong>{entry.kind === "directory" && <small>文件夹</small>}</span>{entry.kind === "directory" && <ChevronRight className="row-chevron" size={15} />}</span><span className="file-meta">{entry.size}</span><span className="file-meta modified-meta">{entry.modified}</span><span className="row-menu" onClick={(event) => { event.stopPropagation(); openContextMenu(event, entry); }}><MoreHorizontal size={16} /></span>
          </button>) : <div className="empty-state"><Search size={20} /><strong>没有匹配的文件</strong><span>试试其他关键词</span></div>}
          <div className="table-footnote"><span className="footnote-dot" />示例目录 <span>·</span> 真实文件访问尚未连接</div>
        </div>
        <footer className="statusbar"><span><span className="status-indicator" />示例模式 · 文件操作待接入</span><span>R2 URL <strong>{rules.baseUrl.replace(/^https?:\/\//, "")}</strong></span></footer>
      </section>

      <aside className="inspector"><div className="inspector-header"><div><span className="section-label">QUICK LOOK</span><h2>预览</h2></div><button className="icon-button" aria-label="更多预览操作" title="更多预览操作" onClick={() => selectedEntry && openContextMenuForButton(selectedEntry)}><MoreHorizontal size={18} /></button></div>
        {selectedEntry ? <><div className={`preview-stage ${selectedEntry.kind !== "image" ? "non-image-stage" : ""}`}>{selectedEntry.kind === "image" && selectedEntry.previewUrl ? <img src={selectedEntry.previewUrl} alt={selectedEntry.name} /> : <div className={`preview-document preview-${selectedEntry.kind}`}><EntryIcon kind={selectedEntry.kind} size={34} /><span>{selectedEntry.name.split(".").pop()?.toUpperCase()}</span><small>{selectedEntry.kind === "video" ? "视频预览接入中" : "快速预览接入中"}</small></div>}<button className="preview-expand" aria-label="打开文件" title="打开文件"><ArrowDownToLine size={15} /></button></div>
          <div className="preview-caption"><span className="filetype-label">{selectedEntry.name.split(".").pop()?.toUpperCase() ?? "FILE"}</span><span className="file-size-label">{selectedEntry.size}</span></div><h3 className="preview-title">{selectedEntry.name}</h3><div className="preview-path"><Folder size={13} /> {parentPath(selectedEntry.path) || "Media Library"}</div>
          <div className="inspector-divider" /><div className="inspector-section-title"><span>R2 OBJECT URL</span><button className="text-action" onClick={openRules}>编辑规则</button></div><div className="url-preview"><span>{buildObjectUrl(selectedEntry.path, rules)}</span></div><button className="copy-link-button" onClick={() => copyLinks(selectedEntry, "url")}><Copy size={15} />复制 URL<ChevronDown size={14} /></button>
          <div className="details-list"><div><span>添加时间</span><strong>Today, 09:12</strong></div><div><span>存储位置</span><strong>本地目录</strong></div><div><span>公开状态</span><strong className="public-state"><span />规则已配置</strong></div></div><button className="share-options" onClick={() => openContextMenuForButton(selectedEntry)}><span><Image size={15} />复制为其他格式</span><ChevronRight size={15} /></button></>
          : <div className="inspector-empty"><Image size={24} /><span>选择文件以查看预览</span></div>}
        <div className="inspector-footer"><span><ShieldCheck size={14} /> 本应用不执行 R2 同步</span></div>
      </aside>

      {menu && <div className="context-menu" style={{ left: menu.x, top: menu.y }} onClick={(event) => event.stopPropagation()}><div className="context-title">复制链接 <span>{menu.entry.kind === "directory" ? "目录" : "文件"}</span></div><button onClick={() => copyLinks(menu.entry, "url")}><Copy size={15} /><span>纯文本 URL</span><kbd>TXT</kbd></button><button onClick={() => copyLinks(menu.entry, "markdown")}><FileText size={15} /><span>Markdown</span><kbd>MD</kbd></button><button onClick={() => copyLinks(menu.entry, "html")}><Archive size={15} /><span>HTML 链接</span><kbd>HTML</kbd></button>{menu.entry.kind === "directory" && <div className="context-hint">将递归复制目录内所有文件链接</div>}</div>}
      {settingsOpen && <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && setSettingsOpen(false)}><form className="settings-modal" onSubmit={saveRules}>
        <div className="modal-heading"><div><span className="section-label">LINK CONFIGURATION</span><h2>URL 规则</h2></div><button type="button" className="icon-button" aria-label="关闭" onClick={() => setSettingsOpen(false)}><X size={18} /></button></div>
        <p className="modal-description">根据目录相对路径生成公开对象 URL。文件同步仍由外部服务负责。</p>
        <label className="field-label">公开 URL 基址<input value={draftRules.baseUrl} onChange={(event) => setDraftRules({ ...draftRules, baseUrl: event.currentTarget.value })} placeholder="https://cdn.example.com" /></label>
        <label className="field-label">对象前缀<input value={draftRules.prefix} onChange={(event) => setDraftRules({ ...draftRules, prefix: event.currentTarget.value })} placeholder="可留空" /></label>
        <label className="field-label">存储桶名称 <span>模板可引用 {"{bucket}"}</span><input value={draftRules.bucket ?? ""} onChange={(event) => setDraftRules({ ...draftRules, bucket: event.currentTarget.value })} placeholder="可选" /></label>
        <label className="field-label">高级模板 <span>可选</span><input value={draftRules.template ?? ""} onChange={(event) => setDraftRules({ ...draftRules, template: event.currentTarget.value })} placeholder="{base}/{key} 或 {base}/{bucket}/{key}" /></label>
        <div className="template-help"><span>{"{base}"}</span>公开基址 <span>{"{key}"}</span>前缀 + 相对路径 <span>{"{bucket}"}</span>存储桶名</div><div className="modal-preview"><span>预览</span><code>{safePreview(selectedEntry?.path ?? "folder/file.jpg")}</code></div>
        <div className="modal-actions"><button type="button" className="button-secondary" onClick={() => setSettingsOpen(false)}>取消</button><button className="button-primary" type="submit"><Check size={15} />保存规则</button></div>
      </form></div>}
      {toast && <div className="toast"><Check size={15} />{toast}</div>}
    </main>
  );
}

export default App;
