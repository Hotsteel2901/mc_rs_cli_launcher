# 验证记录

日期：2026-10-14。

## 最终结果：三平台通过

- 被测代码提交：`6d8cd4889caaee6b00be6cf35ab12683b6f6cbb3`。
- 工作分支：`arena/2bf40672-mc-rs-cli-launcher`，未合并到 `main`。
- [GitHub Actions 完整运行记录](https://github.com/Hotsteel2901/mc_rs_cli_launcher/actions/runs/38543818916)，最终结论 **success**。
- 本报告是在该运行完成后补充的文档，不改变被测代码。

| 验证项目 | Linux | Windows | macOS |
| --- | --- | --- | --- |
| `cargo fmt --all -- --check` | 通过 | 通过 | 通过 |
| 单元测试 | 43 / 43 | 43 / 43 | 43 / 43 |
| CLI 集成测试 | 5 / 5 | 4 / 4 | 5 / 5 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 | 通过 | 通过 |
| `cargo build --locked --release` | 通过 | 通过 | 通过 |

Windows 少一项是因为假 Java shell 脚本启动测试仅在 Unix 上运行，不是运行失败或临时忽略测试。所有测试均无失败。

## 验证过程

沙箱未预装 Rust；通过 npm 的预打包工具链安装后，本地 `cargo test --locked` 仍无法读取 `https://index.crates.io/config.json`（TLS/出网限制）。因此没有把本地完整编译或测试记为通过，而是推送当前工作分支，通过真正的 GitHub 托管 runner 验证。

第一轮验证发现 macOS 临时目录路径别名（`/var` 与 `/private/var`）导致的测试断言问题，以及 Clippy 的多余 let-return 警告；均已修复并重新执行。最后一轮三平台的格式、测试、严格 lint 和 release 构建全部通过。

本地另外执行了：Rustfmt WASM 格式化/语法解析、`git diff --check`，以及 CI 诊断脚本成功/失败退出码冒烟测试。CI 日志下载域名也受沙箱限制，因此增加了通过 GitHub Checks annotations 读取测试摘要和失败详情的机制，没有降低任何检查标准。

## 自动测试范围

- HTTP：本地 TCP fixture 模拟成功、HTTP 503、截断响应和错误 SHA-1；验证错误状态保留、写盘错误传播、临时文件清理、旧文件保留和摘要缓存复用。
- 镜像策略：官方/镜像/自动选项、完整域名匹配、防止相似域名和 URL 参数误替换。
- 账号：模拟钥匙串，验证令牌不写入 JSON、旧明文迁移、钥匙串不可用时拒绝明文回退、logout、用户名校验、损坏 JSON 拒绝覆盖及 Unix 文件权限。
- 元数据：Maven 坐标和扩展名、路径穿越/Windows 保留名、UUID、操作系统与 features 规则、损坏资源 hash、缓存版本及 latest 别名、原生库更新、加载器库覆盖和 profile 父版本校验。
- 模组：启用/禁用/卸载、唯一匹配、歧义查询不改文件、只读列表不创建目录。
- CLI：参数冲突、RAM/线程/窗口大小边界、版本参数区别、离线账号生命周期、无网络列表、损坏账号文件报错。
- 启动：Unix 上用假 Java 验证相对路径、带空格的目录、classpath/游戏目录的绝对路径、用户名、RAM、窗口尺寸、工作目录和子进程退出码。测试 JAR 是自建 fixture，不包含 Minecraft 游戏文件。

## 尚未做的真实环境测试

不能把上述结果等同于以下项目已经通过：

- 真实 Microsoft → Xbox Live → XSTS → Minecraft 授权及刷新；尤其是应用 ID 的服务端授权要求。
- Linux Secret Service、Windows 凭据管理器、macOS Keychain 的真实交互。
- Mojang、BMCLAPI、Modrinth 实际下载和网络故障恢复。
- Fabric / Forge / NeoForge 的真实安装器与 processors 全流程。
- Minecraft 图形客户端、音频/原生库、存档和模组兼容性。

这些需要可访问对应服务、有正确 Java/图形环境和合法账号的机器。建议发布前使用独立测试目录完成下列人工验收，不要直接使用重要存档：

1. 离线账号下载并启动一个现代正式版，检查窗口尺寸、声音、资源和退出码。
2. 故意截断缓存 JAR，再次下载，确认检测损坏并修复。
3. 在系统钥匙串已解锁时进行 Microsoft 登录、重启启动器、会话刷新、logout；确认 JSON 无令牌，钥匙串条目正确清理。
4. 分别在独立目录安装 Fabric / Forge / NeoForge，启动到主菜单；添加兼容模组后检查依赖与加载器选择。
5. 断网后启动文件齐全的指定版本；检查缺文件/过期认证会明确报错，不假装启动成功。

## 仍有的产品边界

同一 Minecraft 版本下多个加载器共享游戏目录；模组依赖处理不是完整冲突求解器；没有跨进程安装锁；旧式不含 `version.json` 的 Forge-family 安装器不支持。文档已明确这些限制，不宣称“所有版本、账号、模组组合都已验证”。
