GPU Monitor 首个 Windows / macOS 安装版。

支持服务器自定义名称、远程 NVIDIA GPU 监控、可选远程 CPU / 系统内存、透明浮窗与历史曲线。Apple 芯片 Mac 可直接监控本机 GPU 利用率及统一内存，Windows 可直接监控本机 NVIDIA GPU。

Mac 菜单栏每张 GPU 显示一个状态圆点：红色忙碌、绿色空闲、灰色离线或数据未知。点击圆点或 Dock 图标打开详细面板；透明浮窗可独立关闭，并已修复切换桌面后消失的问题。

## 下载与安装

- **Windows x64**：下载 `GPU-Monitor_0.1.0_windows-x64-setup.exe` 并运行安装。需要 WebView2；本机 NVIDIA 监控需已安装驱动及 `nvidia-smi`。
- **macOS 11+，Apple 芯片 / Intel 通用版**：下载 `GPU-Monitor_0.1.0_macos-universal.dmg`，打开后把 GPU Monitor 拖到 Applications。本机 GPU 采集仅支持 Apple 芯片；Intel Mac 可监控远程服务器。
- **Agent CLI（可选）**：Windows 下载独立 CLI `.exe`；Mac 下载 CLI `.tar.gz` 并解压。运行 `gpu-monitor-cli --help` 查看用法。
- `SHA256SUMS.txt` 提供各文件的 SHA-256 校验值。

## 签名与当前范围

Windows 安装包未使用商业代码签名；Mac 使用本地签名，尚未通过 Apple 公证，首次打开可能被系统拦截。macOS 可在确认下载来源后，按系统“隐私与安全性”中的提示选择“仍要打开”。

Apple GPU 的内存显示为“GPU 使用 / 共享系统总量”，不代表独立显存或可用额度。GPU 温度、功耗、频率，以及本机 CPU、AMD / Intel 本机 GPU 暂不支持。

安装包经过两平台构建和自动检查；Mac 本机监控已在 Apple M4 验证。首次添加远程服务器时仍需核对 SSH 主机指纹。
