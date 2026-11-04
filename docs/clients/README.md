# 原生客户端设计

本目录存放**多邻灶 Android 原生客户端**的产品与技术设计。

| 文档 | 内容 |
|------|------|
| [webview-shell.md](webview-shell.md) | **2026-11-04 起的交付方向**：系统 WebView 壳 + PWA + 180 天长会话 + 系统 Passkey（PWA/长会话已落地，壳待实现）；替代 design.md 的原生 UI 交付方式 |
| [design.md](design.md) | 原原生设计：第一版为**员工端（竖屏）完整功能**；登录与账户设置与 Web 同步（邮箱登录为 OTP，无 magic link）；注册即登录后绑定 Passkey；只从自家服务器 `/app/` 分发与应用内更新；minSdk 30；工作台日后竖屏设计；鸿蒙暂缓。UI 交付方式见 webview-shell.md |
| [canteen-portrait-pages.md](canteen-portrait-pages.md) | 食堂竖屏状态机：状态列表 / 转移条件 / 跳转边 / 实现对照（不含视觉与布局） |
| [../requirements-phase1.md](../requirements-phase1.md) | 一期需求正文（权威） |
| [../ui/README.md](../ui/README.md) | Web 界面参考；原生信息结构对齐即可 |

**实现代码**计划放在 [`clients/android/`](../../clients/README.md)。**设计确认前不落地业务代码。**

纯血鸿蒙（HarmonyOS NEXT）**本期暂缓**，不建 `clients/harmony/`。

开源仓库禁止写入私有部署信息。示例域名一律 `https://example.com`。
