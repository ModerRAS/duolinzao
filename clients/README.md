# 客户端工程目录

多客户端源码统一放在本目录下，**不要**塞进 `web/`。

| 路径 | 计划 | 状态 |
|------|------|------|
| `android/` | Kotlin + Compose，minSdk 30；包内 `server.json`；登录默认 Passkey（Credential Manager），密码回退；第一版仅员工端；APK 只从自家站点 `/app/` 分发 | **进行中（员工端初版，见 `android/README.md`）** |
| `harmony/` | 纯血鸿蒙 | **本期暂缓，不创建** |

设计文档见 [`docs/clients/`](../docs/clients/README.md)。当前目录仅作占位，不含业务实现。
