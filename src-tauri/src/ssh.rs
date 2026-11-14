use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncWriteExt, process::Command};

pub const GPU_QUERY_ARGS: [&str; 2] = [
    "--query-gpu=index,uuid,name,utilization.gpu,memory.used,memory.total,temperature.gpu",
    "--format=csv,noheader,nounits",
];
pub const INSTALL_KEY: &str = r#"set -eu; umask 077; mkdir -p "$HOME/.ssh"; chmod 700 "$HOME/.ssh"; touch "$HOME/.ssh/authorized_keys"; key=$(tr -d '\r\n'); test -n "$key"; grep -qxF -- "$key" "$HOME/.ssh/authorized_keys" || printf '\n%s\n' "$key" >> "$HOME/.ssh/authorized_keys"; chmod 600 "$HOME/.ssh/authorized_keys""#;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Connection {
    pub target: String,
    pub port: Option<u16>,
    pub identity: Option<String>,
    pub jump: Option<String>,
}

/// Build a connection from form fields, keeping all shell syntax out of input.
pub fn connection_from_fields(host: &str, port: u16, username: &str) -> Result<String, String> {
    let host = host.trim();
    let username = username.trim();
    if host.is_empty() || host.len() > 253 || host.contains('@') {
        return Err("请输入有效的 IP 或域名。".into());
    }
    validate_address(host, false)?;
    if username.is_empty()
        || username.len() > 64
        || username.starts_with('-')
        || !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        return Err("请输入有效的 SSH 用户名。".into());
    }
    if port == 0 {
        return Err("端口必须在 1–65535 之间。".into());
    }
    Ok(format!("ssh -p {port} {username}@{host}"))
}

/// Accept SSH connection arguments only. Never execute a pasted shell command.
pub fn parse_connection(input: &str) -> Result<Connection, String> {
    if input.len() > 4096 || input.contains(['\n', '\r', '\0']) {
        return Err("SSH 命令太长或包含换行。".into());
    }
    let words = shell_words::split(input).map_err(|_| "SSH 命令中的引号不完整。")?;
    if words.first().map(String::as_str) != Some("ssh") {
        return Err("请填写以 ssh 开头的连接命令。".into());
    }
    let mut result = Connection {
        target: String::new(),
        port: None,
        identity: None,
        jump: None,
    };
    let mut username = None;
    let mut i = 1;
    while i < words.len() {
        let word = &words[i];
        if word.starts_with('-') {
            if word.len() < 2 {
                return Err("SSH 参数不完整。".into());
            }
            let flag = ["-p", "-i", "-J", "-l"]
                .into_iter()
                .find(|flag| word.starts_with(flag))
                .ok_or_else(|| {
                    format!("暂不支持参数 {word}；可使用 -p、-i、-J、-l 或 SSH 配置别名。")
                })?;
            let value = if word.len() > 2 {
                word[2..].to_owned()
            } else {
                i += 1;
                words.get(i).cloned().ok_or("SSH 参数缺少值。")?
            };
            if value.is_empty() {
                return Err("SSH 参数不能为空。".into());
            }
            match flag {
                "-p" => {
                    let port: u16 = value.parse().map_err(|_| "端口必须在 1–65535 之间。")?;
                    if port == 0 {
                        return Err("端口不能为 0。".into());
                    }
                    result.port = Some(port);
                }
                "-i" => result.identity = Some(value),
                "-J" => {
                    validate_address(&value, true)?;
                    result.jump = Some(value);
                }
                "-l" => {
                    validate_address(&value, false)?;
                    username = Some(value);
                }
                _ => unreachable!(),
            }
        } else {
            if !result.target.is_empty() {
                return Err("只需连接命令，请移除远程命令或多余内容。".into());
            }
            validate_address(word, false)?;
            result.target = word.clone();
        }
        i += 1;
    }
    if result.target.is_empty() {
        return Err("请提供服务器地址或 SSH 配置别名。".into());
    }
    if let Some(user) = username {
        if result.target.contains('@') {
            return Err("用户名重复，请使用 user@host 或 -l user。".into());
        }
        result.target = format!("{user}@{}", result.target);
    }
    Ok(result)
}

fn validate_address(value: &str, jump: bool) -> Result<(), String> {
    if value.starts_with('-')
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-@[]:%".contains(c) || (jump && c == ','))
    {
        return Err("服务器地址包含不支持的字符。".into());
    }
    Ok(())
}

/// Reuse a local private-key file without opening or returning its contents.
pub fn identity_path(connection: &Connection, home: &Path) -> PathBuf {
    if let Some(path) = &connection.identity {
        return path
            .strip_prefix("~/")
            .or_else(|| path.strip_prefix("~\\"))
            .map(|p| home.join(p))
            .unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(path) = ["gpu_monitor_ed25519", "id_ed25519", "id_rsa", "id_ecdsa"]
        .iter()
        .map(|name| home.join(".ssh").join(name))
        .find(|path| path.is_file())
    {
        return path;
    }
    let mut paired_keys: Vec<_> = std::fs::read_dir(home.join(".ssh"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "pub"))
        .map(|path| path.with_extension(""))
        .filter(|path| path.is_file())
        .collect();
    paired_keys.sort();
    paired_keys
        .into_iter()
        .next()
        .unwrap_or_else(|| home.join(".ssh/gpu_monitor_ed25519"))
}

pub fn command(binary: &str) -> Command {
    #[cfg(target_os = "windows")]
    let path = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
        .join("System32/OpenSSH")
        .join(format!("{binary}.exe"));
    #[cfg(not(target_os = "windows"))]
    let path = PathBuf::from("/usr/bin").join(binary);
    let mut cmd = Command::new(path);
    cmd.kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW: authentication belongs to the app.
    cmd
}

pub fn connection_command(
    connection: &Connection,
    home: &Path,
    interactive: bool,
) -> Result<Command, String> {
    connection_command_with_options(connection, home, interactive, &[])
}

pub fn setup_command(
    connection: &Connection,
    home: &Path,
    target: Option<&PasswordTarget>,
) -> Result<Command, String> {
    authentication_command(connection, home, true, target)
}

pub fn verification_command(
    connection: &Connection,
    home: &Path,
    target: Option<&PasswordTarget>,
) -> Result<Command, String> {
    authentication_command(connection, home, false, target)
}

fn authentication_command(
    connection: &Connection,
    home: &Path,
    interactive: bool,
    target: Option<&PasswordTarget>,
) -> Result<Command, String> {
    let Some(target) = target else {
        // Installation and verification must not reuse a master for a different endpoint.
        return connection_command_with_options(
            connection,
            home,
            interactive,
            &["-o", "ControlPath=none"],
        );
    };
    let fixed = Connection {
        // Retain the alias for Host/IdentityFile settings, but freeze the resolved recipient.
        target: connection
            .target
            .rsplit('@')
            .next()
            .unwrap_or_default()
            .into(),
        port: Some(target.port),
        identity: connection.identity.clone(),
        jump: None,
    };
    connection_command_with_options(
        &fixed,
        home,
        interactive,
        &[
            "-o",
            "ControlPath=none",
            "-o",
            "CanonicalizeHostname=no",
            "-o",
            "ProxyJump=none",
            "-o",
            "ProxyCommand=none",
            "-o",
            &format!("HostName={}", target.hostname),
            "-l",
            &target.user,
        ],
    )
}

fn connection_command_with_options(
    connection: &Connection,
    home: &Path,
    interactive: bool,
    options: &[&str],
) -> Result<Command, String> {
    let mut cmd = command("ssh");
    if interactive {
        cmd.env_remove(crate::askpass::NO_PROMPT_ENV);
    } else {
        let helper = std::env::current_exe().map_err(|e| format!("无法定位免交互认证程序：{e}"))?;
        // ProxyJump's child ssh inherits these even though it does not inherit BatchMode.
        cmd.env("SSH_ASKPASS_REQUIRE", "force")
            .env("SSH_ASKPASS", helper)
            .env(crate::askpass::NO_PROMPT_ENV, "1");
    }
    cmd.args(options);
    cmd.args([
        "-T",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=5",
        "-o",
        "ServerAliveCountMax=2",
        "-o",
        "ClearAllForwardings=yes",
        "-o",
        "PermitLocalCommand=no",
    ]);
    cmd.args(if interactive {
        [
            "-o",
            "StrictHostKeyChecking=ask",
            "-o",
            "NumberOfPasswordPrompts=3",
        ]
    } else {
        ["-o", "StrictHostKeyChecking=yes", "-o", "BatchMode=yes"]
    });
    if let Some(port) = connection.port {
        cmd.args(["-p", &port.to_string()]);
    }
    if let Some(jump) = &connection.jump {
        cmd.args(["-J", jump]);
    }
    let identity = identity_path(connection, home);
    if identity.is_file() {
        cmd.arg("-i").arg(identity);
    }
    cmd.arg(&connection.target);
    Ok(cmd)
}

#[derive(Debug, PartialEq)]
pub struct PasswordTarget {
    hostname: String,
    user: String,
    port: u16,
}

impl PasswordTarget {
    pub fn prompt(&self) -> String {
        format!("{}@{}'s password: ", self.user, self.hostname)
    }
}

/// Resolve aliases locally; uncertain or proxied connections require an explicit app prompt.
pub async fn direct_password_target(
    connection: &Connection,
    home: &Path,
) -> Option<PasswordTarget> {
    let cmd = connection_command_with_options(connection, home, true, &["-G"]).ok()?;
    let config = output(cmd, None, 5).await.ok()?;
    password_target_from_config(&config)
}

fn password_target_from_config(config: &str) -> Option<PasswordTarget> {
    let mut user = None;
    let mut hostname = None;
    let mut port = None;
    for line in config.lines() {
        let (key, value) = line.split_once(char::is_whitespace)?;
        let value = value.trim();
        match key {
            "proxyjump" | "proxycommand" if !value.is_empty() && value != "none" => return None,
            "user" => {
                if user.replace(value).is_some() {
                    return None;
                }
            }
            "hostname" => {
                if hostname.replace(value).is_some() {
                    return None;
                }
            }
            "port" => {
                if port.replace(value.parse::<u16>().ok()?).is_some() {
                    return None;
                }
            }
            _ => {}
        }
    }
    let (user, hostname, port) = (user?, hostname?, port?);
    if user.is_empty()
        || user.contains(['\r', '\n', '\0'])
        || hostname.is_empty()
        || hostname.starts_with('-')
        || !hostname
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-:[]".contains(c))
        || port == 0
    {
        return None;
    }
    Some(PasswordTarget {
        hostname: hostname.into(),
        user: user.into(),
        port,
    })
}

/// Bound the entire process lifetime, including a stalled authenticated server.
pub async fn output(
    mut cmd: Command,
    input: Option<Vec<u8>>,
    timeout: u64,
) -> Result<String, String> {
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().map_err(|e| format!("无法启动 OpenSSH：{e}"))?;
    let task = async {
        if let Some(bytes) = input {
            child
                .stdin
                .take()
                .ok_or("SSH 输入通道不可用。")?
                .write_all(&bytes)
                .await
                .map_err(|e| e.to_string())?;
        }
        let result = child.wait_with_output().await.map_err(|e| e.to_string())?;
        if !result.status.success() {
            let error = String::from_utf8_lossy(&result.stderr).trim().to_string();
            return Err(if error.is_empty() {
                "SSH 执行失败。".into()
            } else {
                error.chars().take(1800).collect()
            });
        }
        String::from_utf8(result.stdout).map_err(|_| "服务器返回了无效文本。".into())
    };
    tokio::time::timeout(Duration::from_secs(timeout), task)
        .await
        .map_err(|_| "连接超时，请检查网络或重新接入。".to_string())?
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub index: u32,
    pub uuid: String,
    pub name: String,
    pub utilization: Option<f64>,
    pub memory_used: Option<f64>,
    pub memory_total: Option<f64>,
    pub temperature: Option<f64>,
}

impl Gpu {
    pub fn available(&self) -> bool {
        matches!((self.utilization, self.memory_used, self.memory_total), (Some(u), Some(m), Some(t)) if u < 10.0 && t > 0.0 && m / t < 0.1)
    }
}

pub fn parse_gpus(text: &str) -> Result<Vec<Gpu>, String> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let mut gpus = Vec::new();
    for record in reader.records() {
        let row = record.map_err(|e| format!("GPU 数据格式异常：{e}"))?;
        if row.len() != 7 {
            return Err("GPU 数据字段不完整。".into());
        }
        let utilization = number(&row[3])?;
        if utilization.is_some_and(|v| v > 100.0) {
            return Err("GPU 利用率超出范围。".into());
        }
        gpus.push(Gpu {
            index: row[0].parse().map_err(|_| "GPU 编号无效。")?,
            uuid: row[1].to_owned(),
            name: row[2].to_owned(),
            utilization,
            memory_used: number(&row[4])?,
            memory_total: number(&row[5])?,
            temperature: number(&row[6])?,
        });
    }
    if gpus.is_empty() {
        return Err("未检测到 NVIDIA GPU。请确认已安装 NVIDIA 驱动和 nvidia-smi。".into());
    }
    Ok(gpus)
}

/// A due batch contains only the GPU UUIDs whose own deadline has elapsed.
pub async fn query_selected(
    connection: &Connection,
    home: &Path,
    uuids: Option<&[String]>,
) -> Result<Vec<Gpu>, String> {
    let mut cmd = connection_command(connection, home, false)?;
    let mut query = format!("LC_ALL=C nvidia-smi {}", GPU_QUERY_ARGS.join(" "));
    if let Some(selection) = gpu_selection_argument(uuids)? {
        query.push(' ');
        query.push_str(&selection);
    }
    cmd.arg(query);
    parse_selected_gpus(&output(cmd, None, 25).await?, uuids)
}

/// A selection is one data argument locally and safe literal text in the remote command.
pub fn gpu_selection_argument(uuids: Option<&[String]>) -> Result<Option<String>, String> {
    if let Some(uuids) = uuids {
        if uuids.is_empty()
            || uuids.iter().any(|id| {
                !id.starts_with("GPU-")
                    || id.len() <= 4
                    || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
        {
            return Err("GPU 标识无效，请重新读取设备列表。".into());
        }
        Ok(Some(format!("--id={}", uuids.join(","))))
    } else {
        Ok(None)
    }
}

pub fn parse_selected_gpus(text: &str, uuids: Option<&[String]>) -> Result<Vec<Gpu>, String> {
    let gpus = parse_gpus(text)?;
    if let Some(uuids) = uuids {
        if uuids.iter().any(|id| !gpus.iter().any(|g| &g.uuid == id)) {
            return Err("部分 GPU 已不可见，请点击立即刷新重新发现设备。".into());
        }
    }
    Ok(gpus)
}

fn number(value: &str) -> Result<Option<f64>, String> {
    if ["N/A", "[N/A]", "[Not Supported]", "Not Supported"].contains(&value) {
        return Ok(None);
    }
    let number: f64 = value
        .parse()
        .map_err(|_| format!("无效 GPU 指标：{value}"))?;
    if !number.is_finite() || number < 0.0 {
        return Err("GPU 指标不是有效的非负数。".into());
    }
    Ok(Some(number))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_selection_rejects_arguments_and_requires_every_requested_card() {
        assert_eq!(gpu_selection_argument(None).unwrap(), None);
        let selected = vec!["GPU-first".into(), "GPU-second".into()];
        assert_eq!(
            gpu_selection_argument(Some(&selected)).unwrap().as_deref(),
            Some("--id=GPU-first,GPU-second")
        );
        assert!(gpu_selection_argument(Some(&[])).is_err());
        for invalid in [
            "GPU-",
            "--help",
            "GPU-first,GPU-second",
            "GPU-first;whoami",
            "GPU-first\n",
        ] {
            assert!(gpu_selection_argument(Some(&[invalid.into()])).is_err());
        }
        let first = "0, GPU-first, NVIDIA Test, 12, 256, 1024, 40\n";
        assert!(parse_selected_gpus(first, Some(&selected)).is_err());
        let both = format!("{first}1, GPU-second, NVIDIA Test, N/A, 0, 2048, 32\n");
        let gpus = parse_selected_gpus(&both, Some(&selected)).unwrap();
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].utilization, Some(12.0));
        assert_eq!(gpus[1].utilization, None);
    }

    #[test]
    fn form_fields_cannot_smuggle_ssh_options() {
        assert_eq!(
            connection_from_fields("10.0.0.1", 22, "root").unwrap(),
            "ssh -p 22 root@10.0.0.1"
        );
        assert!(connection_from_fields("server;reboot", 22, "root").is_err());
        assert!(connection_from_fields("server", 22, "root -o X").is_err());
        assert!(connection_from_fields("server", 0, "root").is_err());
        assert!(connection_from_fields("2001:db8::1", 22, "alice").is_ok());
    }
    #[test]
    fn connection_arguments_are_data() {
        let c = parse_connection("ssh -p 2222 root@example.com").unwrap();
        assert_eq!(c.port, Some(2222));
        assert_eq!(
            parse_connection("ssh -l alice -J jump lab").unwrap().target,
            "alice@lab"
        );
        assert_eq!(parse_connection("ssh -p2200 lab").unwrap().port, Some(2200));
        for bad in [
            "ssh host;calc",
            "ssh host reboot",
            "ssh -o ProxyCommand=calc host",
            "ssh -p 0 host",
            "ssh -p 65536 host",
            "ssh host\nwhoami",
            "ssh -L 22 host",
            "ssh $(calc)",
        ] {
            assert!(parse_connection(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn automatic_password_requires_resolved_direct_configuration() {
        let direct = "user alice\nhostname example.com\nport 2222\n";
        let target = password_target_from_config(direct).unwrap();
        assert_eq!(
            target,
            PasswordTarget {
                user: "alice".into(),
                hostname: "example.com".into(),
                port: 2222,
            }
        );
        assert_eq!(target.prompt(), "alice@example.com's password: ");
        assert!(
            password_target_from_config(&format!("{direct}proxyjump bastion.example.com\n"))
                .is_none()
        );
        assert!(password_target_from_config(&format!(
            "{direct}proxycommand ssh -W %h:%p bastion.example.com\n"
        ))
        .is_none());
        for invalid in [
            "hostname example.com\nport 22\n",
            "user alice\nhostname \nport 22\n",
            "user alice\nhostname example.com\n",
            "user alice\nhostname example.com\nport 0\n",
            "user alice\nhostname example.com\nport 65536\n",
            "user alice\nhostname %h\nport 22\n",
            "user alice\nhostname other@example.com\nport 22\n",
        ] {
            assert!(password_target_from_config(invalid).is_none(), "{invalid}");
        }
        assert!(password_target_from_config(&format!("{direct}port 2203\n")).is_none());
        assert!(password_target_from_config(&format!(
            "{direct}proxyjump none\nproxycommand none\n"
        ))
        .is_some());
    }

    #[test]
    fn automatic_password_pins_the_same_endpoint_for_installation_and_verification() {
        let connection = parse_connection("ssh -p 9999 original@lab").unwrap();
        let target =
            password_target_from_config("user alice\nhostname example.com\nport 2222\n").unwrap();
        let home = Path::new("unused-test-home");
        let args = |cmd: Command| {
            cmd.as_std()
                .get_args()
                .map(|s| s.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        let setup = args(setup_command(&connection, home, Some(&target)).unwrap());
        let verify = args(verification_command(&connection, home, Some(&target)).unwrap());
        for command in [&setup, &verify] {
            for option in [
                "ControlPath=none",
                "CanonicalizeHostname=no",
                "ProxyJump=none",
                "ProxyCommand=none",
                "HostName=example.com",
            ] {
                assert!(command.windows(2).any(|s| s == ["-o", option]));
            }
            assert!(command.windows(2).any(|s| s == ["-l", "alice"]));
            assert!(command.windows(2).any(|s| s == ["-p", "2222"]));
            assert_eq!(command.iter().filter(|arg| *arg == "-p").count(), 1);
            assert!(!command
                .iter()
                .any(|arg| arg == "9999" || arg == "original@lab" || arg == "-J"));
            // Matching Host lab preserves its identity and host-key settings.
            assert_eq!(command.last().map(String::as_str), Some("lab"));
        }
        assert!(setup.iter().any(|s| s == "StrictHostKeyChecking=ask"));
        assert!(verify.iter().any(|s| s == "StrictHostKeyChecking=yes"));
        assert!(verify.iter().any(|s| s == "BatchMode=yes"));
    }

    #[test]
    fn manual_authentication_keeps_proxy_settings_but_never_reuses_a_master() {
        let home = Path::new("unused-test-home");
        let jump = parse_connection("ssh -J bastion.example.com alice@example.com").unwrap();
        for command in [
            setup_command(&jump, home, None).unwrap(),
            verification_command(&jump, home, None).unwrap(),
        ] {
            let args: Vec<_> = command
                .as_std()
                .get_args()
                .map(|s| s.to_string_lossy())
                .collect();
            assert!(args.windows(2).any(|s| s == ["-J", "bastion.example.com"]));
            assert!(args.windows(2).any(|s| s == ["-o", "ControlPath=none"]));
            assert!(!args.iter().any(|s| s == "ProxyJump=none"));
            assert_eq!(args.last().map(|s| s.as_ref()), Some("alice@example.com"));
        }
    }

    #[test]
    fn background_prompts_are_rejected_by_an_inherited_helper_policy() {
        let connection = parse_connection("ssh -J bastion.example.com alice@example.com").unwrap();
        let home = Path::new("unused-test-home");
        let environment = |cmd: Command| {
            cmd.as_std()
                .get_envs()
                .map(|(key, value)| (key.to_owned(), value.map(|value| value.to_owned())))
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let env = environment(connection_command(&connection, home, false).unwrap());
        let key = |name: &str| std::ffi::OsStr::new(name).to_owned();
        assert_eq!(
            env.get(&key("SSH_ASKPASS_REQUIRE")),
            Some(&Some("force".into()))
        );
        assert_eq!(
            env.get(&key("SSH_ASKPASS")),
            Some(&Some(std::env::current_exe().unwrap().into_os_string()))
        );
        assert_eq!(
            env.get(&key(crate::askpass::NO_PROMPT_ENV)),
            Some(&Some("1".into()))
        );
        let env = environment(setup_command(&connection, home, None).unwrap());
        assert_eq!(env.get(&key(crate::askpass::NO_PROMPT_ENV)), Some(&None));
    }
    #[test]
    fn parse_real_units_and_unavailable_metrics() {
        let gpus = parse_gpus("0, GPU-abc, NVIDIA RTX 4090, 93, 20480, 24564, 65\n1, GPU-def, NVIDIA A100, [N/A], 1024, 81920, N/A\n").unwrap();
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].memory_used, Some(20480.0));
        assert!(gpus[1].utilization.is_none());
        assert!(!gpus[1].available());
        assert!(parse_gpus("0,GPU-x,\"GPU, Name\",1,0,1024,30").unwrap()[0].available());
        for bad in [
            "",
            "not a csv",
            "0,x,gpu,NaN,0,1024,30",
            "0,x,gpu,101,0,1024,30",
        ] {
            assert!(parse_gpus(bad).is_err());
        }
    }

    #[test]
    fn reuses_custom_keys_without_reading_private_contents() {
        let home = std::env::temp_dir().join(format!("gpu-monitor-test-{}", uuid::Uuid::new_v4()));
        let dir = home.join(".ssh");
        std::fs::create_dir_all(&dir).unwrap();
        let connection = parse_connection("ssh host").unwrap();
        assert_eq!(
            identity_path(&connection, &home),
            dir.join("gpu_monitor_ed25519")
        );
        std::fs::write(dir.join("my_cluster"), "not read by discovery").unwrap();
        std::fs::write(dir.join("my_cluster.pub"), "public placeholder").unwrap();
        assert_eq!(identity_path(&connection, &home), dir.join("my_cluster"));
        std::fs::write(dir.join("id_ed25519"), "default placeholder").unwrap();
        assert_eq!(identity_path(&connection, &home), dir.join("id_ed25519"));
        let explicit = parse_connection("ssh -i ~/custom_key host").unwrap();
        assert_eq!(identity_path(&explicit, &home), home.join("custom_key"));
        std::fs::remove_dir_all(&home).unwrap();
    }
}
