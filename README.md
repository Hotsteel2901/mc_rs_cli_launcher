# Hot-RS-Launcher

Rust 编写的 Minecraft 命令行启动器。支持 Microsoft / 离线账号、按 Minecraft 版本隔离游戏目录、Fabric / Forge / NeoForge、Java 运行时管理、实例重命名/复制/删除、Modrinth 模组更新、.mrpack 整合包导入导出、崩溃诊断，以及备份恢复。内置可选的全屏终端前端 `hot-rs-launcher tui`，仍然只依赖 CLI 核心，不引入 GUI。

> 离线账号不提供正版认证，不能进入要求正版认证的服务器。Microsoft 登录需要拥有 Minecraft Java Edition 的账号。

- 作者：Hotsteel2901
- GitHub：<https://github.com/Hotsteel2901/mc_rs_cli_launcher>
- Codeberg：<https://codeberg.org/hotsteel/mc_rs_cli_launcher>

两个平台上的仓库同名（`mc_rs_cli_launcher`），issue / PR 任选其一即可。运行 `hot-rs-launcher --about` 或 TUI 的「关于」页也能看到这些信息。

## 构建

安装当前 stable Rust 工具链后：

```sh
cargo build --locked --release
./target/release/hot-rs-launcher --help
./target/release/hot-rs-launcher --launcher-version
./target/release/hot-rs-launcher --about
```

Windows 使用 `target\release\hot-rs-launcher.exe`。Java 会按版本元数据查找，启动游戏时可尝试下载 Mojang runtime；推荐事先安装适合该 Minecraft 版本的 Java，或通过 `--java` 指定。Forge / NeoForge 安装器需要已安装的 Java。

## 快速开始

```sh
# 所有命令使用相同的 --dir；相对路径和带空格的路径均可
hot-rs-launcher offline Steve --dir ./minecraft
hot-rs-launcher download -v 1.21.4 --dir ./minecraft
hot-rs-launcher play -v 1.21.4 --ram 4G --dir ./minecraft

# 可选的全屏终端界面（同一二进制）
hot-rs-launcher tui --dir ./minecraft

# Microsoft 账号
hot-rs-launcher login --dir ./minecraft
hot-rs-launcher login --device-code --dir ./minecraft
hot-rs-launcher accounts --dir ./minecraft
hot-rs-launcher logout --dir ./minecraft

# Java、实例与整合包
hot-rs-launcher list-java --dir ./minecraft
hot-rs-launcher install-java --java-major 21 --dir ./minecraft
hot-rs-launcher rename-version -v 1.21.4 --name my-pack --dir ./minecraft
hot-rs-launcher export-modpack -v my-pack -o my-pack.mrpack --dir ./minecraft
hot-rs-launcher import-modpack my-pack.mrpack --dir ./minecraft
```

默认目录是可执行文件所在目录下的 `minecraft/`。`-v` / `--version` 指 **Minecraft 版本**；启动器版本使用 `--launcher-version`。`latest` 和 `latest-snapshot` 分别解析最新正式版和快照版。省略命令等同 `play`。

## TUI 前端（可选）

```sh
hot-rs-launcher tui
```

同一个 CLI 二进制里的键盘/鼠标驱动全屏界面，包含动态 Dashboard、远程版本选择与下载、加载器选择与安装、Modrinth 搜索/安装、模组启停与更新检查、账号管理、实例重命名/复制/删除/导出整合包、设置（语言、内存、窗口、下载源、Java 运行时选择、JVM 参数、GC 预设）、活动日志和「关于」页（`7` 直达，`g` / `b` 可打开 GitHub / Codeberg）。文件下载有分阶段进度、实时速度、每文件百分比和可取消的任务面板；界面支持 30 FPS 渐变/脉冲/Toast/粒子动效；空闲 2 秒后暂停环境动画，终端输出从约 118 KiB/s 降到 1 KiB/s 以下，并可在设置中关闭开场动画和减少动效。

- 需要交互式终端和 ANSI 支持；stdin/stdout 不是 TTY 或 `TERM=dumb` 时会明确报错，不会在 CI 或管道中挂起。
- 支持中英文界面切换（默认 English），在 Settings 第一项即时切换并自动保存。
- 支持鼠标点击侧栏/标签、主按钮、列表行、设置项和弹窗按钮，滚轮可移动选择；点击列表行选中，双击执行主要动作。
- 设置中的 Java 行会列出本机检测到的运行时并标注推荐项；模组页按 `u` 检查更新、按 `U` 批量应用，列表用 `↑` 标记可更新项；启动失败后崩溃诊断会写入活动日志。
- 下载默认 16 并发、复用 HTTP/TLS 连接；资源下载使用更宽的并发池（默认 32，最高 64，BMCLAPI 上限 24），并在 `auto` 模式下先测速再决定用官方还是镜像。
- 窗口宽度小于 96 列自动切换顶部标签布局；小于 52×16 显示尺寸提示。
- 终端始终在退出、错误、挂起和主线程 panic 时恢复；运行任务退出时会被取消并清理子进程树。
- 配置保存在游戏目录的 `launcher_tui.json`，不含令牌。
- Microsoft 登录和游戏启动会临时离开备用屏幕，让钥匙串回退确认、设备码和游戏控制台正常显示；界面列表只读取账号元数据，只有登录或启动时 CLI 核心才访问钥匙串，令牌从不出现在界面中。
- 完整按键表、截图、配置字段、架构与验证方式见 **[docs/TUI.md](docs/TUI.md)**。

TUI 是可选的便利层：所有自动化、脚本与服务器场景都可以继续只使用下面的 CLI 命令。

### 账号安全

- JSON 只保存用户名、UUID、账号类型和到期时间。Microsoft 访问/刷新令牌优先保存在操作系统钥匙串中，并按游戏目录区分；令牌从不以明文写入磁盘。
- 钥匙串服务名已随改名切换为 `hot-rs-launcher`；升级后首次读取会自动把旧 `mc-launcher` 条目迁移过来，`logout` 会同时清理两个名字，不需要重新登录。
- Linux 桌面需要可用且已解锁的 Secret Service / 系统钥匙串。若钥匙串不可用（例如无 D-Bus 的无头环境），启动器会询问是否改用本地加密文件存储：
  - 交互式终端中会显示 `[y/N]` 提示，仅在回答 `y`/`yes` 后才启用；之后该游戏目录会记住此选择（下次直接使用，不再询问）。
  - 非交互环境请设置 `MC_LAUNCHER_ALLOW_FILE_STORE=1` 表示同意、`=0` 表示拒绝；未设置且非交互时不会回退，并给出明确错误。
  - 本地回退把令牌写入 `launcher_secrets.enc`，用随机密钥 `launcher_secrets.key`（权限 0600）做加密+完整性校验。由于密钥与密文放在一起，其安全性弱于系统钥匙串，仅用于无法使用钥匙串的机器。删除 `launcher_secrets.key` 即可回到“仅使用系统钥匙串”的行为。
- `logout` 总是清除本地账号元数据，即使钥匙串不可访问也一样；此时会打印警告，提示手动删除残留的钥匙串条目。登出不会为了删除而创建本地密钥/同意文件。
- 旧版明文账号文件会在读取时迁移；安全存储写入失败时保留原文件并报错，不静默丢失账号，也不写回明文。迁移前存在的备份仍可能包含敏感信息，请自行妥善保管。
- `logout` 清理本启动器本地记录与相应钥匙串条目，不等于在 Microsoft 服务端撤销授权。
- 浏览器登录校验返回地址和随机 OAuth `state`。不要分享回调 URL、设备验证码或账号文件。
- 可用 `MC_LAUNCHER_CLIENT_ID` 配置自己的 Microsoft 公共客户端应用 ID。应用需获准访问相应 Xbox/Minecraft API；内置历史客户端 ID 的可用性由 Microsoft 决定，设备码流程还要求应用允许 public-client/device flow。自动测试不验证真实账号授权。

> 兼容性说明：`MC_LAUNCHER_*` 环境变量和 `launcher_accounts.json` / `launcher_tui.json` / `launcher_secrets.*` 文件名保持不变，升级不需要迁移游戏目录。

### 下载与缓存

```sh
hot-rs-launcher download -v 1.21.4 --threads 8 --official
hot-rs-launcher download -v 1.21.4 --bmcl
hot-rs-launcher download -v 1.21.4 --no-assets
```

- 默认：官方优先，对支持映射的公共资源失败后尝试 BMCLAPI；不再用 ping 判断地区。
- 资源（assets）阶段有单独的调度：`auto` 模式先用 2 个小型资源对象同时探测官方与 BMCLAPI，整批走更快的一方；资源并发默认是 `--threads` 的两倍（至少 16，最多 64），镜像上限 24，以避开 BMCLAPI 对高并发的限流。失败的文件会在最后单独重试一轮，不会因为一个 TLS 抖动而让整批资源失败。
- `--official`：启动器自身请求不使用镜像。`--bmcl`：已支持的域名只走镜像，其他域名仍走官方（例如 NeoForge）。两者互斥。
- 登录请求及携带额外请求头的请求不经过镜像。Forge-family **Java 安装器内部的网络请求由安装器控制**，不受这些选项约束。
- 下载先写同目录临时文件，检查读取/写入错误、长度和元数据提供的 SHA-1，再替换目标。无摘要的 JAR/JSON 检查结构。失败保留原文件，不发布半成品。
- 客户端、库和资源的已有缓存会重新校验可用摘要。指定已安装版本可以不访问全局版本列表；离线启动仍要求所需文件齐全，Microsoft 账号仍须有有效会话。
- `--threads` 范围 1–64（默认 16）；`--ram` 范围 128–65536 MB（TUI 与 CLI 一致）；`--no-assets` 仅用于 `download`，后续正常启动仍会补全资源。

### 加载器和模组

```sh
hot-rs-launcher install-fabric -v 1.21.4
hot-rs-launcher install-forge -v 1.20.1 --java /path/to/java
hot-rs-launcher install-neoforge -v 1.21.4
hot-rs-launcher play -v 1.21.4 --fabric --ram 4G --width 1280 --height 720

hot-rs-launcher search sodium --loader fabric -v 1.21.4
hot-rs-launcher search-more sodium
hot-rs-launcher install-mod sodium --loader fabric -v 1.21.4
hot-rs-launcher list-mods -v 1.21.4
hot-rs-launcher disable-mod sodium.jar -v 1.21.4
hot-rs-launcher enable-mod sodium.jar.disabled -v 1.21.4
hot-rs-launcher uninstall-mod sodium.jar -v 1.21.4
```

可用 `--loader-version` / `--mod-version` 指定加载器/模组版本。`--fabric`、`--forge`、`--neoforge`、`--loader` 互斥。没有显式选择时，保留自动检测加载器行为（Fabric、NeoForge、Forge 顺序）。

Forge 和 NeoForge 执行官方安装器及其 processors，不再把“只解压 JAR”当作完整安装。旧式不含 `version.json` 的安装器不支持。旧版启动器保存的 Forge/NeoForge profile 需要重新运行相应安装命令，避免继续复用不完整安装。模组依赖会显示提示，必需依赖可交互确认安装；目前不是完整的依赖冲突求解器。安装模组或执行加载器安装器意味着运行第三方代码，仅使用可信项目。

禁用/启用/卸载优先匹配完整文件名；模糊查询必须唯一，歧义时不修改任何文件；不会覆盖已有目标。`list-mods` 不会创建空目录。

## Java 管理

```sh
hot-rs-launcher list-java                 # 已安装的 JDK/JRE（版本、厂商、来源）
hot-rs-launcher list-java -v 1.21.4       # 标注该版本推荐的 Java
hot-rs-launcher install-java --java-major 21
hot-rs-launcher play -v 1.21.4 --java /path/to/java
```

- `list-java` 读取每个 JDK 的 `release` 文件（必要时回退到 `java -version`），并区分 `JAVA_HOME`、`PATH`、系统扫描和 `<game_dir>/java` 下的 Mojang runtime。`-v` 会按该版本的 `javaVersion.majorVersion` 标出推荐项。
- `install-java` 从 Mojang 的 runtime 索引中选择覆盖该主版本的最新组件，下载到 `<game_dir>/java/`，校验 SHA-1 后可直接用于启动；重复执行会复用已安装的运行时。
- 启动时优先选择与版本要求精确匹配的本机 JDK，其次才下载 Mojang runtime，最后回退到系统 Java。用 `--java` 显式指定时会提示版本不符。

## 实例管理

```sh
hot-rs-launcher list-installed
hot-rs-launcher rename-version -v 1.21.4 --name my-pack
hot-rs-launcher copy-version -v my-pack --name my-pack-b
hot-rs-launcher remove-version -v my-pack-b --keep-saves
hot-rs-launcher backup -v my-pack --with-mods
hot-rs-launcher list-backups
hot-rs-launcher restore-backup ./backups/my-pack-20250101-120000.zip
```

- 重命名/复制会同步版本 JSON、客户端 JAR、加载器 profile 和 natives 目录，并在实例目录写入 `instance.json` 记录原始 Minecraft 版本；加载器 profile 查找和整合包导出依赖这条信息。
- `remove-version` 删除实例目录、加载器 profile 和 natives；`--keep-saves` / `--keep-mods` 可以保留存档或模组目录。CLI 不会交互确认，删除前请确认 `-v` 指向正确实例。
- 备份是 ZIP：包含 `saves/`、`config/`、`options.txt`、`servers.dat`、实例元数据和版本 JSON；`--with-mods` 额外包含 `mods/`。恢复默认写回备份中记录的实例，`-v` 可指定目标实例。

## 整合包（Modrinth .mrpack）

```sh
hot-rs-launcher export-modpack -v my-pack -o my-pack.mrpack
hot-rs-launcher import-modpack my-pack.mrpack
hot-rs-launcher import-modpack my-pack.mrpack --name my-pack --install-loader
```

- 导出时用 SHA-1 查询 Modrinth：能识别的模组写成 `downloads` 条目，其余（本地模组、禁用模组）放进 `overrides/mods/`，因此离线也能导出一个可用的包。
- 导入会创建独立实例，下载 `files` 中的模组（校验 SHA-1），安全解压 `overrides/` 与 `client-overrides/`，并记录 Minecraft 版本和加载器。基础版本元数据缺失时会自动获取，但不会留下一个空的“已安装版本”。
- `--install-loader` 会在导入后安装包声明的加载器；不指定时只打印对应命令，Forge / NeoForge 安装器仍需要可用的 Java。

## 模组更新与崩溃诊断

```sh
hot-rs-launcher update-mods -v 1.21.4                # 只检查
hot-rs-launcher update-mods -v 1.21.4 --apply        # 下载并替换
hot-rs-launcher update-mods -v 1.21.4 --apply --all  # 包含已禁用模组
hot-rs-launcher diagnose -v 1.21.4
```

- 更新检查同样按 SHA-1 识别已安装 JAR，再查询当前 Minecraft 版本与加载器下最新的 Modrinth 版本；非 Modrinth 来源的 JAR 会显示为“无法识别”，不会被改动。已禁用的模组更新后保持禁用。
- 启动失败（退出码非 0）时会自动读取最新崩溃报告或 `logs/latest.log`，识别内存不足、Java 版本不符、模组依赖冲突、Mixin 失败、显卡/GLFW、登录会话、网络、磁盘/权限等常见问题并给出处理建议。`diagnose` 可随时手动执行。

## 启动参数

```sh
hot-rs-launcher play -v 1.21.4 --gc g1
hot-rs-launcher play -v 1.21.4 --jvm-args "-Dfoo=bar -XX:+UseStringDeduplication"
hot-rs-launcher play -v 1.21.4 --server mc.example.com:25565
hot-rs-launcher play -v 1.21.4 --fullscreen --width 1920 --height 1080
hot-rs-launcher play -v 1.21.4 --dry-run
```

- `--gc`：`auto`（默认）、`g1`（Aikar 风格参数）、`zgc`（Java 21 自动启用分代）、`parallel`、`serial`、`none`。
- `--jvm-args` 按 shell 风格拆分（支持引号），作为独立 argv 传入，不经过 shell。
- `--server` 支持 `host`、`host:port` 和 `[IPv6]:port`；非法地址在参数解析阶段就会拒绝。
- `--dry-run` 不下载缺失文件、不刷新令牌、不启动游戏，只打印完整命令行，便于排查参数问题；`--no-diagnose` 关闭启动失败后的自动分析。

## 目录结构

```text
minecraft/
  launcher_accounts.json       # 非敏感账号元数据
  launcher_tui.json            # 可选：TUI 偏好（语言、内存、窗口、源、Java 等）
  launcher_secrets.enc         # 可选：钥匙串不可用时的加密令牌（需用户同意）
  launcher_secrets.key         # 可选：上述加密文件的本地密钥（0600）
  versions/<id>/               # 该实例的 saves、mods、config、日志及客户端
    instance.json              # 可选：重命名/复制/整合包实例的基准版本与加载器
  libraries/                   # 共享库和加载器 profile
  assets/                      # 资源索引、对象及旧版虚拟资源
  natives/<id>/                # 原生库
  java/                        # 自动下载的 Java runtime
  backups/                     # backup 命令生成的 ZIP
  modpacks/                    # export-modpack 的默认输出目录
```

同一 Minecraft 版本下不同加载器仍共享游戏目录；切换加载器前请备份存档并检查 mods 兼容性。不要同时对同一个目录运行多个安装/账号写入操作。

## 开发与验证

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

CI 在 Linux、Windows、macOS 执行上述命令。测试使用本地 HTTP fixture 和模拟钥匙串，不需要正版账号；Unix 上用假的 Java 可执行文件验证离线启动参数、工作目录和子进程退出码。TUI 使用 `TestBackend` 在多种终端尺寸下渲染全部界面（含「关于」页）、弹窗和任务状态，并验证中英文切换、鼠标热区和 Java/GC/更新相关界面；开发验证中还使用伪终端冒烟测试检查备用屏幕、括号粘贴、鼠标捕获开关、实例重命名对话框与按键退出。实例生命周期、备份恢复和 .mrpack 往返都有离线集成测试；模组更新只对纯比较逻辑做单元测试，真实 Modrinth 调用需要网络。非 TTY 下的 `tui` 会明确失败，不会阻塞 CI。这不是实际 Minecraft 图形界面或 Microsoft 服务的端到端测试。参见 [验证记录](docs/VALIDATION.md) 与 [TUI 说明](docs/TUI.md)。

## 许可证

沿用项目声明的 [`GPL-3.0-only`](LICENSE)，许可证全文取自 SPDX license-list-data 的 GPL-3.0-only 文本。本轮改动为原创修复，未复制其他启动器实现；依赖保留各自许可证。
