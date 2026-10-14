# 验证记录

日期：2026-10-14。

- 沙箱未预装 Rust；通过 npm 上的预打包工具链安装了 Rust 1.92 nightly 用于尝试本地构建。
- 本地 `cargo test --locked` 无法读取 `https://index.crates.io/config.json`（TLS/出网限制），因此没有把本地编译或测试记为通过。
- 所有 Rust 文件已经过 Rustfmt WASM 格式化和语法解析；`git diff --check` 通过。
- 当前工作分支配置了 Linux / Windows / macOS 的 GitHub Actions：格式、单元/CLI 集成测试、Clippy、release 构建。实际结果将在运行结束后更新。

## 测试范围

本地 HTTP 服务模拟成功、503、截断响应、错误校验和；覆盖磁盘写错误传播、临时文件清理、旧文件保留、镜像域名边界。模拟钥匙串覆盖令牌不落盘、旧账号迁移、logout、损坏 JSON 拒绝覆盖。规则和路径测试覆盖 Maven 坐标、UUID、操作系统/功能规则、危险路径。CLI 集成测试覆盖离线账号生命周期、无网络列表、错误参数、Unix 假 Java 启动与退出码。

## 不能据此宣称通过的项目

真实 Microsoft/Xbox/Minecraft 授权、真实钥匙串跨平台集成、Mojang/BMCLAPI/Modrinth 下载、Fabric/Forge/NeoForge 全流程安装，以及 Minecraft 图形客户端启动。需要在可访问对应服务、有正确 Java/图形环境和合法账号的机器上实测。
