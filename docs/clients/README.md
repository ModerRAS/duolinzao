# 原生客户端设计

本目录存放**多邻灶 Android 原生客户端**的产品与技术设计。

| 文档 | 内容 |
|------|------|
| [design.md](design.md) | 完整设计：第一版为**员工端（竖屏）完整功能**；登录与账户设置与 Web 同步（邮箱登录为 OTP，无 magic link）；注册即登录后绑定 Passkey；只从自家服务器 `/app/` 分发与应用内更新；minSdk 30；工作台日后竖屏设计；鸿蒙暂缓 |
| [../requirements-phase1.md](../requirements-phase1.md) | 一期需求正文（权威） |
| [../ui/README.md](../ui/README.md) | Web 界面参考；原生信息结构对齐即可 |

**实现代码**计划放在 [`clients/android/`](../../clients/README.md)。**设计确认前不落地业务代码。**

纯血鸿蒙（HarmonyOS NEXT）**本期暂缓**，不建 `clients/harmony/`。

开源仓库禁止写入私有部署信息。示例域名一律 `https://example.com`。
