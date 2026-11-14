# GPU Monitor

目的：提供 macOS 状态栏与 Windows 托盘小窗，直接读取本机 NVIDIA GPU，或通过 SSH 按 GPU 展示远程利用率和显存，可选远程 CPU 与系统内存；在独立设置页按资源管理采样间隔与历史长度。

保持小而美：优先局部修改与系统内置能力，不为简单需求引入框架或泛用层。主面板白底，蓝色利用率、橙色显存细线；设置页仅白灰色。顶栏只保留设置、添加、刷新、收起。

界面在 `src/`，桌面实现与采集在 `src-tauri/src/`。不在服务器部署后台服务；不保存用户密码；不执行粘贴命令中的任意 shell 内容；不得禁用 SSH 主机指纹检查。

本机入口为 **＋ → 本机**，调用本机 `nvidia-smi`，无需 SSH、密钥或密码。与远程 GPU 共用刷新、历史、浮窗和 CLI；CLI 的 `source` 区分 `local` 与 `ssh`，现有 `--server` 可按 ID／完整名称选择。本机 CPU 与 Apple／AMD／Intel 本机 GPU 暂不支持；默认本机不采集 CPU，CLI 显式 `--cpu` 返回采集错误与退出码 2。

入口：`npm run desktop`。前端检查：`npm run build && npm test`。后端检查：`cargo test --manifest-path src-tauri/Cargo.toml`。构建命令见 README。

输出：网页构建在 `dist/`；桌面构建在 `src-tauri/target/`；Windows 本机便携副本在 `app/`，通过 `scripts/install-local.ps1` 更新并创建桌面入口。运行配置在系统应用配置目录，不写入仓库。前端演示数据只在浏览器 `?demo` 模式启用。

本项目不启用 Python 注释审计标准。若后续需要层级声明，建议 `layers: [["src", "src-tauri"]]`；不为此创建额外目录。
