# mc-launcher

Rust 编写的 Minecraft 命令行启动器。支持 Microsoft / 离线账号、按 Minecraft 版本隔离游戏目录、Fabric / Forge / NeoForge，以及 Modrinth 模组管理。

> 离线账号不提供正版认证，不能进入要求正版认证的服务器。Microsoft 登录需要拥有 Minecraft Java Edition 的账号。

## 构建

安装当前 stable Rust 工具链后：

```sh
cargo build --locked --release
./target/release/mc-launcher --help
./target/release/mc-launcher --launcher-version
```

Windows 使用 `target\release\mc-launcher.exe`。Java 会按版本元数据查找，启动游戏时可尝试下载 Mojang runtime；推荐事先安装适合该 Minecraft 版本的 Java，或通过 `--java` 指定。Forge / NeoForge 安装器需要已安装的 Java。

## 快速开始

```sh
# 所有命令使用相同的 --dir；相对路径和带空格的路径均可
mc-launcher offline Steve --dir ./minecraft
mc-launcher download -v 1.21.4 --dir ./minecraft
mc-launcher play -v 1.21.4 --ram 4G --dir ./minecraft

# Microsoft 账号
mc-launcher login --dir ./minecraft
mc-launcher login --device-code --dir ./minecraft
mc-launcher accounts --dir ./minecraft
mc-launcher logout --dir ./minecraft
```

默认目录是可执行文件所在目录下的 `minecraft/`。`-v` / `--version` 指 **Minecraft 版本**；启动器版本使用 `--launcher-version`。`latest` 和 `latest-snapshot` 分别解析最新正式版和快照版。省略命令等同 `play`。

### 账号安全

- JSON 只保存用户名、UUID、账号类型和到期时间。Microsoft 访问/刷新令牌仅保存在操作系统钥匙串中，并按游戏目录区分。
- Linux 桌面需要可用且已解锁的 Secret Service / 系统钥匙串。无钥匙串的无头环境可以使用离线账号，但 Microsoft 登录不会退回到明文存储。
- 旧版明文账号文件会在读取时迁移；钥匙串写入失败时保留原文件并报错，不静默丢失账号。迁移前存在的备份仍可能包含敏感信息，请自行妥善保管。
- `logout` 清理本启动器本地记录与相应钥匙串条目，不等于在 Microsoft 服务端撤销授权。
- 浏览器登录校验返回地址和随机 OAuth `state`。不要分享回调 URL、设备验证码或账号文件。
- 可用 `MC_LAUNCHER_CLIENT_ID` 配置自己的 Microsoft 公共客户端应用 ID。应用需获准访问相应 Xbox/Minecraft API；内置历史客户端 ID 的可用性由 Microsoft 决定，设备码流程还要求应用允许 public-client/device flow。自动测试不验证真实账号授权。

### 下载与缓存

```sh
mc-launcher download -v 1.21.4 --threads 8 --official
mc-launcher download -v 1.21.4 --bmcl
mc-launcher download -v 1.21.4 --no-assets
```

- 默认：官方优先，对支持映射的公共资源失败后尝试 BMCLAPI；不再用 ping 判断地区。
- `--official`：启动器自身请求不使用镜像。`--bmcl`：已支持的域名只走镜像，其他域名仍走官方（例如 NeoForge）。两者互斥。
- 登录请求及携带额外请求头的请求不经过镜像。Forge-family **Java 安装器内部的网络请求由安装器控制**，不受这些选项约束。
- 下载先写同目录临时文件，检查读取/写入错误、长度和元数据提供的 SHA-1，再替换目标。无摘要的 JAR/JSON 检查结构。失败保留原文件，不发布半成品。
- 客户端、库和资源的已有缓存会重新校验可用摘要。指定已安装版本可以不访问全局版本列表；离线启动仍要求所需文件齐全，Microsoft 账号仍须有有效会话。
- `--threads` 范围 1–32；`--no-assets` 仅用于 `download`，后续正常启动仍会补全资源。

### 加载器和模组

```sh
mc-launcher install-fabric -v 1.21.4
mc-launcher install-forge -v 1.20.1 --java /path/to/java
mc-launcher install-neoforge -v 1.21.4
mc-launcher play -v 1.21.4 --fabric --ram 4G --width 1280 --height 720

mc-launcher search sodium --loader fabric -v 1.21.4
mc-launcher search-more sodium
mc-launcher install-mod sodium --loader fabric -v 1.21.4
mc-launcher list-mods -v 1.21.4
mc-launcher disable-mod sodium.jar -v 1.21.4
mc-launcher enable-mod sodium.jar.disabled -v 1.21.4
mc-launcher uninstall-mod sodium.jar -v 1.21.4
```

可用 `--loader-version` / `--mod-version` 指定加载器/模组版本。`--fabric`、`--forge`、`--neoforge`、`--loader` 互斥。没有显式选择时，保留自动检测加载器行为（Fabric、NeoForge、Forge 顺序）。

Forge 和 NeoForge 执行官方安装器及其 processors，不再把“只解压 JAR”当作完整安装。旧式不含 `version.json` 的安装器不支持。旧版启动器保存的 Forge/NeoForge profile 需要重新运行相应安装命令，避免继续复用不完整安装。模组依赖会显示提示，必需依赖可交互确认安装；目前不是完整的依赖冲突求解器。安装模组或执行加载器安装器意味着运行第三方代码，仅使用可信项目。

禁用/启用/卸载优先匹配完整文件名；模糊查询必须唯一，歧义时不修改任何文件；不会覆盖已有目标。`list-mods` 不会创建空目录。

### 目录结构

```text
minecraft/
  launcher_accounts.json       # 非敏感账号元数据
  versions/<mc-version>/       # 该版本的 saves、mods、config、日志及客户端
  libraries/                   # 共享库和加载器 profile
  assets/                      # 资源索引、对象及旧版虚拟资源
  natives/<mc-version>/        # 原生库
  java/                        # 自动下载的 Java runtime
```

同一 Minecraft 版本下不同加载器仍共享游戏目录；切换加载器前请备份存档并检查 mods 兼容性。不要同时对同一个目录运行多个安装/账号写入操作。

## 开发与验证

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

CI 在 Linux、Windows、macOS 执行上述命令。测试使用本地 HTTP fixture 和模拟钥匙串，不需要正版账号；Unix 上用假的 Java 可执行文件验证离线启动参数、工作目录和子进程退出码。这不是实际 Minecraft 图形界面或 Microsoft 服务的端到端测试。参见 [验证记录](docs/VALIDATION.md)。

## 许可证

沿用项目声明的 [`GPL-3.0-only`](LICENSE)，许可证全文取自 SPDX license-list-data 的 GPL-3.0-only 文本。本轮改动为原创修复，未复制其他启动器实现；依赖保留各自许可证。
