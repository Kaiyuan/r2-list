# R2 List

R2 List 是一个基于 Tauri、Rust、React 和 TypeScript 的桌面文件工作区，用目录相对路径生成 Cloudflare R2 对象 URL，并支持复制 URL、Markdown 和 HTML 链接。

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
npm run build:portable:linux
```

每个命令只适用于对应操作系统，产物写入 `artifacts/portable/`。推送任意 Git tag 后，GitHub Actions 会构建 Windows x64、macOS（runner 原生架构）和 Linux x64 产物，并创建或更新该 tag 对应的 GitHub Release。

平台运行时、系统依赖及未签名应用的限制见 [`docs/portable-releases.md`](docs/portable-releases.md)。

验证前端：

```sh
npm run build
npm test
```

当前工作区先以样例目录展示交互；本地和远程文件系统适配器尚未接入。应用只负责分析目录映射和生成 URL，不负责向 R2 上传或同步对象。
