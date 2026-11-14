# R2 List

R2 List 是一个基于 Rust 和 Slint 的原生桌面文件工作区，用目录相对路径生成 Cloudflare R2 对象 URL，并支持本地目录及 WebDAV 位置。

Slint 原生界面支持浏览和搜索文件、导入文件、新建文件夹、文本预览，以及复制 URL 或 Markdown 链接。URL 规则和最近使用的位置沿用原有配置文件。

项目主页：[GitHub Pages](https://kaiyuan.github.io/r2-list/)

## 开发环境

- Rust 1.98 或更新的 stable 工具链
- NASM（仅 x86/x64 目标需要，用于编译 AVIF 解码器；Windows 构建脚本会自动下载，Intel Mac 用 `brew install nasm`，Ubuntu x64 用 `apt install nasm`）
- Windows：Visual Studio C++ Build Tools
- macOS：Xcode Command Line Tools
- Linux：Slint 所需的图形/字体开发库（完整依赖列表见 `docs/portable-releases.md`）

## 启动

```sh
cargo run --manifest-path desktop/Cargo.toml
```

## 免安装构建

在对应系统安装 Rust stable、系统构建依赖和对应平台的打包工具后，运行：

```sh
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-portable-windows.ps1
bash scripts/build-portable-macos.sh
bash scripts/build-portable-macos.sh x64 10.15
bash scripts/build-portable-linux.sh x64
```

每个命令只适用于对应操作系统，产物写入 `artifacts/portable/`。`build:portable:macos` 构建当前 Mac 架构，`build:portable:macos:x64` 构建兼容 macOS 10.15 Catalina 的 Intel 版本。推送 `v` 开头的版本 tag（例如 `v1.2.3`）后，GitHub Actions 会构建 Windows x64、macOS arm64、macOS x64（macOS 10.15 Catalina）和 Linux x64/arm64 产物，并创建或更新该 tag 对应的 GitHub Release。普通代码 push 不会触发发布或 Pages 部署。

平台运行时、系统依赖及未签名应用的限制见 [`docs/portable-releases.md`](docs/portable-releases.md)。

验证 Rust 应用：

```sh
cargo fmt --manifest-path desktop/Cargo.toml -- --check
cargo test --manifest-path desktop/Cargo.toml
```

应用首次启动会打开“文档”目录；也可以通过路径输入或系统文件选择器打开本地目录，或连接 WebDAV 服务器。目录内容由 Rust 读取，支持导航、搜索、新建文件夹、导入文件以及在本地/WebDAV 位置内复制和移动；导入或复制时遇到重名会自动生成新文件名，不覆盖原文件。当前使用的位置会保存在本机，后续启动自动恢复。

右侧面板显示所选文件并提供 URL、Markdown 和 HTML 复制、预览、外部打开及删除操作。支持 UTF-8 文本预览（最大 2 MB）、图片预览及 DOCX/XLSX/PPTX 文本提取预览；PDF、音视频和其他不支持内嵌显示的格式可使用系统默认应用打开。URL 规则、位置标签、主题及配置目录可在设置中调整。

WebDAV 由应用内置的 Rust HTTP 客户端直接访问，不依赖操作系统挂载；地址、用户名和密码保存在既有用户配置中，应用重启后会自动重新连接。应用只生成和复制 Cloudflare R2 URL，不负责向 R2 上传或同步对象。界面和后台均由 Rust 与 Slint 实现。
