# R2 List

R2 List 是一个基于 Tauri、Rust、React 和 TypeScript 的桌面文件工作区，用目录相对路径生成 Cloudflare R2 对象 URL，并支持复制 URL、Markdown 和 HTML 链接。

界面支持 English、Français、简体中文、繁體中文、日本語和 한국어；外观可跟随系统设置，或手动选择亮色/暗色。

![R2 List](https://github.com/Kaiyuan/r2-list/blob/main/docs/images/r2list-windows.png)

项目主页：[GitHub Pages](https://kaiyuan.github.io/r2-list/)

## 开发环境

- Node.js 20.19+ 或 22.12+
- Rust stable 工具链
- Windows：Visual Studio C++ Build Tools 与 WebView2 Runtime

## 启动

```sh
npm install
npm run dev
```

运行桌面壳：

```sh
npm run tauri dev
```

## 免安装构建

在对应系统安装 Node.js、Rust stable 和平台构建依赖后，运行：

```sh
npm run build:portable:windows
npm run build:portable:macos
npm run build:portable:macos:x64
npm run build:portable:linux
```

每个命令只适用于对应操作系统，产物写入 `artifacts/portable/`。`build:portable:macos` 构建当前 Mac 架构，`build:portable:macos:x64` 构建兼容 macOS 10.15 Catalina 的 Intel 版本。推送 `v` 开头的版本 tag（例如 `v1.2.3`）后，GitHub Actions 会构建 Windows x64、macOS arm64、macOS x64（macOS 10.15 Catalina）和 Linux x64/arm64 产物，并创建或更新该 tag 对应的 GitHub Release。普通代码 push 不会触发发布或 Pages 部署。

平台运行时、系统依赖及未签名应用的限制见 [`docs/portable-releases.md`](docs/portable-releases.md)。

验证前端：

```sh
npm run build
npm test
cargo test --manifest-path src-tauri/Cargo.toml
```

应用首次启动会打开“文档”目录；也可以通过左侧位置选择其他本地目录。目录内容由 Rust 后端读取，支持导航、搜索、新建文件夹和导入文件；导入时遇到重名会自动生成新文件名，不覆盖原文件。所选目录会保存在本机，后续启动自动恢复。

双击文件可打开大预览，右侧面板显示当前选中文件。支持常见图片、UTF-8 文本、Markdown、CSV/TSV、PDF、DOCX、XLSX、PPTX，以及 WebView 可解码的音频/视频。文本预览限制为 2 MB，Office 文件限制为 16 MB；大图片、PDF 和音视频通过受限的本地文件 URL 读取。任意专有/二进制格式、旧版 Office 格式以及当前 WebView 不支持的媒体 codec 无法保证预览。

支持本地目录和 WebDAV 服务器；WebDAV 由应用内置的 Rust HTTP 客户端直接访问，不依赖操作系统 WebDAV 挂载。添加位置时输入服务器地址及登录信息；WebDAV 地址、用户名和密码会保存在用户配置中，应用重启后会自动重新连接。应用只生成和复制 Cloudflare R2 URL，不负责向 R2 上传或同步对象。
