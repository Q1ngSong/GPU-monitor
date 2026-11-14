# GPU Monitor

把本机和远程 GPU 的状态，放到桌面一角。

一个小而简洁的桌面监控工具。直接读取 Apple 芯片 Mac 的本机 GPU 或 Windows/Linux 的 NVIDIA GPU，也可以通过 SSH 连接多台服务器，查看每张 GPU 的利用率、显存和历史趋势。提供 Agent 可以直接读取的 JSON 接口。

白色面板、蓝色利用率曲线、橙色显存曲线。Mac 菜单栏每张 GPU 一个状态圆点，点击展开详情，也可保留透明浮窗。

**Windows 已完成本机验证；macOS 已完成 Apple 芯片与 Intel 通用版编译，并在 Apple 芯片 Mac 上验证启动、本机 Apple M4 与远程 GPU 显示。Intel 真机与多屏行为仍待验证。**

## 功能

| 功能 | 说明 |
| --- | --- |
| 本机 GPU | Apple 芯片 Mac / Windows、Linux 的 NVIDIA GPU，无需 SSH 或密码 |
| 多服务器监控 | 每张 GPU 独立显示型号、利用率、显存和温度 |
| 历史折线 | 默认最近 1 小时，可调整为 15 分钟至 24 小时 |
| Mac 菜单栏 | 每张 GPU 一个圆点：红色忙碌、绿色空闲、灰色离线或数据未知；点击打开详情 |
| 透明浮窗 | 始终置顶、可调透明度，鼠标移入时隐藏内容并显示拖动柄 |
| 独立刷新 | 利用率越低，默认更新越快；支持全局默认与单卡覆盖 |
| CPU 监控 | 可选远程服务器 CPU 利用率、系统内存及历史趋势 |
| Agent CLI | 按需读取实时状态、历史与配置，返回 JSON |

使用 Tauri、Rust 和原生 TypeScript，复用系统 WebView 与 OpenSSH。服务器无需安装常驻监控程序。

## 快速开始

在 [GitHub Releases](https://github.com/Q1ngSong/GPU-monitor/releases/latest) 下载 Windows x64 安装程序或 macOS 通用 DMG。Mac 打开 DMG 后将 App 拖到 Applications；Windows 运行安装程序。安装包签名与首次打开说明见 Release 正文，也可从源码[开发与构建](#开发与构建)。

**添加本机 GPU**：打开 App，点击右上角 **＋ → 本机**。Apple 芯片 Mac 直接读取系统 GPU 统计，无需管理员密码；Windows/Linux 需有 NVIDIA GPU、驱动及可用的 `nvidia-smi`。无需填写 SSH、端口、用户名或密码。本机 GPU 使用相同的刷新设置、历史曲线、透明浮窗和 CLI。本机 CPU、AMD 和 Intel GPU 暂不支持。

**Apple GPU 指标**：显示型号、利用率、GPU 正在使用的统一内存及历史。内存格式为 **GPU 使用 / 系统总量**，总量由 CPU 与 GPU 共享，不是独立显存容量或 GPU 可用额度；没有可靠读数的温度等指标保持未知。采集来自 Apple 驱动的只读统计，已在 Apple M4 上验证；这些驱动字段可能随 macOS 版本变化。

**添加远程服务器**：本机需有 OpenSSH；远端需为 Linux，通过 `nvidia-smi` 提供 NVIDIA GPU 指标。CPU 与内存指标来自 `/proc`。

1. 打开 App，点击右上角 **＋**，选择远程服务器。
2. 填写 **IP／域名、端口、用户名、密码**。已有免密连接时，密码可以留空。
3. 首次连接时核对服务器指纹，在 App 内完成认证。
4. App 验证免密连接后开始监控。默认开启 GPU，远程 CPU 可在设置中开启。

添加远程服务器时，App 会检测本机 SSH 密钥；没有可用密钥时，创建 `~/.ssh/gpu_monitor_ed25519`，并把公钥追加到服务器账户的 `authorized_keys`。已有内容会保留。

顶部仅保留 **设置、添加、刷新、收起**。点击设置打开面板，按钮保持按下状态；再次点击返回主页面。关闭主窗口会收起到托盘，监控继续运行；从托盘菜单选择退出才会停止。Mac 也可以点击 Dock 图标或从“应用程序”重新打开面板。

**Mac 菜单栏圆点**：按设备列表及 GPU 顺序排列，超过 8 个时排成两行。悬停查看设备名称与状态，点击打开详细面板，右键打开操作菜单。NVIDIA 的利用率与显存占用都低于 10% 时为绿色；Apple GPU 只按利用率低于 10% 判断空闲。断线、采样过期或指标未知时为灰色；首次连接尚未识别 GPU 数量的设备暂用一个灰点占位。关闭某设备的 GPU 监控后，其圆点也会移除。

**自定义设备名称**：点击设备标题右侧的 **⋯ → 重命名**，输入 `autodl`、`训练机` 等代号并保存。名称会同步显示在主面板、浮窗、设置页和 CLI 中，重启后保留；连接地址、密钥和历史记录不变。

## 透明浮窗

浮窗默认开启，以简洁数值显示 GPU 利用率和显存，以及已启用的 CPU 利用率。

Mac 上浮窗与拖动柄会随桌面切换保持显示；可在设置中关闭浮窗，只保留菜单栏圆点。

- **平时**：显示内容，不显示拖动柄。
- **鼠标移入**：内容隐藏，只留下小柄；拖动小柄即可移动位置。
- **鼠标移出**：内容恢复，小柄隐藏。
- **点击穿透**：除小柄外，点击和滚轮作用于下方窗口。浮窗高度随内容自动变化，无需滚动。

在 **设置 → 透明浮窗** 中调整文字透明度、背景透明度、字号与显示设备。背景不透明度设为 **0%** 即完全透明。

点击 **调整宽度** 后，从边缘调整宽度，高度随内容自动变化；完成后恢复自动隐藏和鼠标穿透。位置与宽度会保存在本机。

## 刷新与历史

设置分为 **监控内容、GPU、CPU**，各面板单独保存。全局设置作为默认值，服务器与单张 GPU 可以设置例外，也可以随时恢复跟随全局。

GPU 默认按当前利用率安排下一次采样：

| GPU 利用率 | 等待间隔 |
| --- | --- |
| 低于 10% | 5 秒 |
| 10% 至低于 60% | 15 秒 |
| 60% 及以上，或指标未知 | 30 秒 |

支持改为固定间隔，或分别调整这三个间隔，范围为 **2–3600 秒**。CPU 使用独立的固定间隔。实际两次采样之间还包含查询耗时。

历史长度可选 **15 分钟、30 分钟、1 小时、3 小时、6 小时、24 小时**。记录保存在本机，重启后恢复；延长窗口不会补齐过去未采集的数据。断线或长时间缺失采样时，曲线留空，后台自动重试。

**“低负载”** 针对独立显存 GPU，要求利用率与显存占用都低于 10%，Apple 统一内存 GPU 不参与该统计，仅表示当前采样状态，不代表 GPU 已被预留。GPU 利用率也不等于 FLOPS 或训练速度。

## Agent CLI

Agent 可以直接调用本机 CLI，读取结构化数据，无需操作界面。CLI 与 App 使用同一系统用户的配置，远程连接复用该用户的 SSH 密钥；本机 GPU 直接采集，桌面窗口无需打开。

Windows 完成下方本机安装后，在项目根目录运行：

```powershell
.\app\gpu-monitor-cli.exe status --json
.\app\gpu-monitor-cli.exe status --server "服务器 ID 或完整名称" --cpu --gpu --json
```

macOS 本机构建后：

```sh
./src-tauri/target/release/gpu-monitor-cli status --json
```

Agent 在其他工作目录执行时，请使用可执行文件的完整路径。安装脚本不会自动添加全局命令。

| 子命令 | 返回内容 |
| --- | --- |
| `servers --json` | 已保存的本机与远程服务器、ID 和 `source`（`local`／`ssh`） |
| `status --json` | 立即查询本机或远程的最新读数 |
| `history --json` | App 已保存的历史，不启动额外采样 |
| `settings --json` | 全局默认、服务器与单卡覆盖项、浮窗设置 |
| `keys --json` | 默认密钥发现结果：路径及文件是否存在 |
| `--help` | 完整用法 |

`status` 默认按配置选择 CPU／GPU，本机默认不查询 CPU；`--cpu`、`--gpu` 只改变本次查询。显式为本机指定 `--cpu` 会返回不支持的采集错误及退出码 `2`。`status` 和 `history` 均支持用 `--server` 选择本机或远程条目的 ID／完整名称，名称重复时请使用 ID。

正常结果包含 `schemaVersion`、`sampledAt`、`data` 三个顶层字段，当前结构版本为 `1`。GPU 数据包括编号、UUID、型号、利用率、内存已用／总量和温度；未知指标为 `null`。`unifiedMemory: true` 表示 Apple 统一内存，此时 `memoryUsed` 为 GPU 正在使用的内存，`memoryTotal` 为共享的系统物理内存总量；历史样本也保留此标记。**CLI 的内存与显存单位为 MiB，利用率为百分比，时间为 Unix 秒；界面显示为 GiB。** 设置中的刷新间隔为秒，历史长度为分钟。

退出码：**0** 成功，**1** 参数或配置错误，**2** 至少一个采集器失败。部分采集失败时仍返回其他结果；参数或配置错误返回 `{ "schemaVersion": 1, "error": "..." }`。

CLI 是只读接口，不添加监控来源、不修改设置、不启动 GPU 任务，也不返回密码或私钥内容。首次添加在 App 内完成；远程查询使用免密认证，每个采集器最多等待 25 秒。

## SSH 与本机数据

- 密码仅用于认证，不写入配置、日志、命令参数或环境变量。
- 新生成的专用密钥不设口令，以支持后台采样；私钥保留在本机。已有加密密钥需先在系统 `ssh-agent` 中解锁。
- 首次连接需要核对主机指纹，不跳过 SSH 身份检查。需要密码的跳板会在 App 内单独提示；后台采样要求各跳板也能免密连接。
- 可通过本机 SSH 配置设置身份文件、跳板或主机别名。底层连接解析支持 `-p`、`-i`、`-l`、`-J`，不执行输入中的任意 shell 命令。
- 连接和设置保存在系统应用配置目录下的 `dev.gpumonitor.desktop/servers.json`，历史保存在同目录的 `history.json`。
- 从 App 移除服务器会删除本机监控配置及其历史，不会撤销服务器上的 SSH 公钥。

源码仓库排除本机配置、历史、密钥、日志和构建产物。

CLI 的服务器、设置和历史输出会包含连接信息或设备标识；分享这些输出前请先脱敏。

## 开发与构建

需要 **Node.js 24+、Rust 1.90**；远程监控还需系统 OpenSSH。Rust 版本由项目固定。Windows 需 Visual Studio C++ Build Tools 与 WebView2，macOS 需 Xcode Command Line Tools；详细环境见 [Tauri 文档](https://v2.tauri.app/start/prerequisites/)。Windows 的 OpenSSH 需为支持 ASKPASS 的 8.6+ 版本。

在项目根目录启动开发版：

```sh
npm ci
npm run desktop
```

仅预览界面：运行 `npm run dev`，打开 `http://127.0.0.1:1420/?demo`。演示页使用模拟数据，不访问 SSH 服务器。

检查与打包：

```sh
npm test
npm run build
rustup component add rustfmt
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --locked --manifest-path src-tauri/Cargo.toml
npm run package
```

前端测试使用 Node 内置测试功能。发布请使用 `npm run package`（仅 CLI 为 `npm run package:cli`），构建时会将 Rust 源路径中的本机用户目录、依赖目录和项目目录替换为通用路径，减少编译产物携带的本机信息。打包会构建桌面 App 与独立 CLI：

| 产物 | 本机构建位置 |
| --- | --- |
| 桌面程序 | `src-tauri/target/release/gpu-monitor`，Windows 带 `.exe` |
| Agent CLI | `src-tauri/target/release/gpu-monitor-cli`，Windows 带 `.exe` |
| 安装包 | `src-tauri/target/release/bundle/` |

在 macOS 上构建 Apple 芯片与 Intel 通用版：

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
APPLE_SIGNING_IDENTITY=- npm run package -- --target universal-apple-darwin --bundles app,dmg -- --locked
```

通用产物位于 `src-tauri/target/universal-apple-darwin/release/`，其中 DMG 位于 `bundle/dmg/`。打包前会自动合并两种架构的独立 CLI，同时将它包含在 App 内。上述命令使用本地临时签名，不包含 Apple 公证；对外分发时需配置自己的开发者签名与公证。

Windows 构建完成后，可整理为便携目录并创建桌面快捷方式：

```powershell
powershell -File scripts/install-local.ps1 -DesktopShortcut -Open
```

程序会放到 `app/`，已有连接、设置和历史继续保留。

GitHub 工作流在 `main` 提交和 PR 时运行 Windows／macOS 检查；手动触发时额外构建、验证 Windows 安装包、macOS 通用 DMG 和独立 CLI。推送 `vX.Y.Z` 标签时，两平台全部通过后自动发布正式 Release，并附带 SHA-256 校验文件。通用 macOS 产物位于 `src-tauri/target/universal-apple-darwin/release/`。

发布前先手动运行一次工作流确认安装包检查通过，再在干净且与远端同步的 `main` 上运行 `npm run release -- X.Y.Z`。首次发布可沿用当前版本，后续版本须递增；脚本会检查、同步版本、提交并推送标签。发布说明位于 `.github/release-notes.md`。当前 Mac 使用本地签名且未公证，Windows 未使用商业代码签名；应用尚未接入自动更新。

## 当前范围

支持 Apple 芯片 Mac 本机 GPU、Windows/Linux 本机 NVIDIA GPU，以及 Linux 远端服务器的 NVIDIA GPU、CPU／系统内存监控。本机 CPU、AMD／Intel 本机 GPU、AMD／Intel 远程 GPU、MIG 实例级统计、进程列表与开机自启尚未实现。Apple GPU 暂不提供温度、功耗和频率。

macOS 已验证本地签名完整性；Intel 真机、实际多屏行为、开发者签名与公证仍待验证。当前仓库尚未添加开源许可证。

开发约定见 [PROJECT.md](PROJECT.md)。
