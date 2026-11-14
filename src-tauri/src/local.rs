use crate::ssh;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;

const UNSUPPORTED: &str =
    "本机监控目前支持 Windows/Linux 上的 NVIDIA GPU，暂不支持 Apple、AMD 或 Intel GPU。";

#[cfg(any(target_os = "windows", target_os = "linux", test))]
fn path_candidates(path: Option<&std::ffi::OsStr>, binary: &str) -> Vec<PathBuf> {
    path.into_iter()
        .flat_map(std::env::split_paths)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(binary))
        .collect()
}

#[cfg(any(target_os = "windows", target_os = "linux", test))]
fn find_executable(
    preferred: Vec<PathBuf>,
    path: Option<&std::ffi::OsStr>,
    binary: &str,
) -> Option<PathBuf> {
    preferred
        .into_iter()
        .chain(path_candidates(path, binary))
        .find(|candidate| candidate.is_absolute() && candidate.is_file())
}

fn find_nvidia_smi() -> Result<PathBuf, String> {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        #[cfg(target_os = "windows")]
        let (preferred, binary) = {
            let absolute_directory = |name: &str, fallback: &str| {
                std::env::var_os(name)
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .unwrap_or_else(|| PathBuf::from(fallback))
            };
            let windows = absolute_directory("WINDIR", "C:\\Windows");
            let program_files = absolute_directory("ProgramFiles", "C:\\Program Files");
            (
                vec![
                    windows.join("System32/nvidia-smi.exe"),
                    program_files.join("NVIDIA Corporation/NVSMI/nvidia-smi.exe"),
                ],
                "nvidia-smi.exe",
            )
        };
        #[cfg(target_os = "linux")]
        let (preferred, binary) = (
            vec![
                PathBuf::from("/usr/bin/nvidia-smi"),
                PathBuf::from("/usr/local/bin/nvidia-smi"),
            ],
            "nvidia-smi",
        );
        find_executable(preferred, std::env::var_os("PATH").as_deref(), binary).ok_or_else(|| {
            format!("未找到本机 nvidia-smi，请确认已安装 NVIDIA 驱动。{UNSUPPORTED}")
        })
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Err(UNSUPPORTED.into())
    }
}

fn query_command(program: &Path, uuids: Option<&[String]>) -> Result<Command, String> {
    let mut command = Command::new(program);
    command
        .args(ssh::GPU_QUERY_ARGS)
        .env("LC_ALL", "C")
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(selection) = ssh::gpu_selection_argument(uuids)? {
        command.arg(selection);
    }
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    Ok(command)
}

/// Query the local driver directly; no shell, SSH credentials or elevated process is involved.
pub async fn query_gpus(uuids: Option<&[String]>) -> Result<Vec<ssh::Gpu>, String> {
    let program = find_nvidia_smi()?;
    let mut command = query_command(&program, uuids)?;
    let output = tokio::time::timeout(Duration::from_secs(25), command.output())
        .await
        .map_err(|_| "本机 GPU 采集超时，请检查 NVIDIA 驱动是否正常。".to_string())?
        .map_err(|error| format!("无法启动本机 NVIDIA 采集：{error}"))?;
    if !output.status.success() {
        let diagnostic = if output.stderr.is_empty() {
            &output.stdout
        } else {
            &output.stderr
        };
        let detail: String = String::from_utf8_lossy(diagnostic)
            .trim()
            .chars()
            .take(1800)
            .collect();
        return Err(if detail.is_empty() {
            format!("本机 nvidia-smi 执行失败，请检查 NVIDIA 驱动。{UNSUPPORTED}")
        } else {
            format!("本机 NVIDIA 采集失败：{detail}")
        });
    }
    let text = String::from_utf8(output.stdout).map_err(|_| "本机 GPU 返回了无效文本。")?;
    ssh::parse_selected_gpus(&text, uuids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_search_ignores_empty_and_relative_directories() {
        let absolute = std::env::temp_dir();
        let path = std::env::join_paths([
            Path::new(""),
            Path::new("."),
            Path::new("tools"),
            absolute.as_path(),
        ])
        .unwrap();
        assert_eq!(
            path_candidates(Some(&path), "nvidia-smi"),
            vec![absolute.join("nvidia-smi")]
        );
    }

    #[test]
    fn preferred_installation_precedes_path_and_directories_are_not_executables() {
        let directory =
            std::env::temp_dir().join(format!("gpu-monitor-local-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let preferred = directory.join("preferred-smi");
        let fallback = directory.join("nvidia-smi");
        std::fs::write(&preferred, b"test fixture; never executed").unwrap();
        std::fs::write(&fallback, b"test fixture; never executed").unwrap();
        let path = std::env::join_paths([&directory]).unwrap();
        assert_eq!(
            find_executable(
                vec![directory.clone(), preferred.clone()],
                Some(&path),
                "nvidia-smi"
            ),
            Some(preferred.clone())
        );
        assert_eq!(
            find_executable(vec![], Some(&path), "nvidia-smi"),
            Some(fallback.clone())
        );
        std::fs::remove_file(preferred).unwrap();
        std::fs::remove_file(fallback).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn gpu_query_passes_selection_as_one_argument_without_a_shell() {
        let program = std::env::temp_dir().join("nvidia-smi");
        let selected = vec!["GPU-first".into(), "GPU-second".into()];
        let command = query_command(&program, Some(&selected)).unwrap();
        assert_eq!(command.as_std().get_program(), program.as_os_str());
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy())
            .collect();
        assert_eq!(
            args,
            [
                ssh::GPU_QUERY_ARGS[0],
                ssh::GPU_QUERY_ARGS[1],
                "--id=GPU-first,GPU-second"
            ]
        );
        assert!(query_command(&program, Some(&["GPU-first; echo bad".into()])).is_err());
    }
}
