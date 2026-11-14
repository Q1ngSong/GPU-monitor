//! Read-only CLI for agents. Uses the same config and collectors as the app.
use crate::{history, now, schedule::Monitor, ssh, Config, ServerConfig, ServerView, Source};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

const HELP: &str = concat!(
    "GPU Monitor CLI ",
    env!("CARGO_PKG_VERSION"),
    "\n\nUSAGE:\n  gpu-monitor-cli servers [--json]\n  gpu-monitor-cli status [--server ID_OR_NAME] [--cpu] [--gpu] [--json]\n  gpu-monitor-cli history [--server ID_OR_NAME] [--json]\n  gpu-monitor-cli settings [--json]\n  gpu-monitor-cli keys [--json]\n  gpu-monitor-cli --help\n\nstatus performs fresh local Apple/NVIDIA GPU or password-free SSH queries (25s timeout per collector).\nSources are queried in parallel. With no resource flags, saved CPU/GPU toggles apply; local CPU is disabled.\n--cpu and --gpu select resources for this call only; they do not change settings. Local CPU is unsupported.\nhistory reads saved samples (up to 24h); settings returns global defaults and saved overrides as JSON.\nPer-GPU and per-server overrides take priority over global defaults.\nThe desktop app need not be running. All commands are read-only.\nPasswords and private-key contents are never returned. Add sources and authenticate remote servers in the app.\n\nEXIT CODES: 0 success; 1 invalid arguments/config; 2 one or more collectors failed.\nJSON: schemaVersion=1; memory values are MiB, utilization is percent, timestamps are Unix seconds. For unifiedMemory=true, memoryUsed is GPU in-use memory and memoryTotal is shared system RAM, not dedicated VRAM.\n"
);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report<T> {
    schema_version: u8,
    sampled_at: u64,
    data: T,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    servers: Vec<ServerView>,
    servers_online: usize,
    gpus_online: usize,
    gpus_low_load: usize,
}

fn summarize(servers: Vec<ServerView>) -> (Status, i32) {
    let servers_online = servers
        .iter()
        .filter(|s| s.status == "online" || (s.cpu.is_some() && s.cpu_error.is_none()))
        .count();
    let gpus_online = servers.iter().map(|s| s.gpus.len()).sum();
    let gpus_low_load = servers
        .iter()
        .flat_map(|s| &s.gpus)
        .filter(|g| g.available())
        .count();
    let exit = if servers
        .iter()
        .all(|s| s.error.is_none() && s.cpu_error.is_none())
    {
        0
    } else {
        2
    };
    (
        Status {
            servers,
            servers_online,
            gpus_online,
            gpus_low_load,
        },
        exit,
    )
}

#[derive(Debug, PartialEq)]
struct Options {
    command: String,
    json: bool,
    server: Option<String>,
    cpu: bool,
    gpu: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let command = args.first().cloned().unwrap_or_else(|| "--help".into());
    if ![
        "--help",
        "-h",
        "--version",
        "servers",
        "status",
        "history",
        "keys",
        "settings",
    ]
    .contains(&command.as_str())
    {
        return Err(format!("Unknown command: {command}. Use --help."));
    }
    let mut options = Options {
        command,
        json: false,
        server: None,
        cpu: false,
        gpu: false,
    };
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => options.json = true,
            "--cpu" if options.command == "status" => options.cpu = true,
            "--gpu" if options.command == "status" => options.gpu = true,
            "--server" if ["status", "history"].contains(&options.command.as_str()) => {
                options.server = Some(
                    args.next()
                        .ok_or("--server requires an ID or exact name")?
                        .clone(),
                )
            }
            _ => return Err(format!("Unknown option: {arg}")),
        }
    }
    Ok(options)
}

fn config_path() -> Result<PathBuf, String> {
    dirs::config_dir()
        .map(|p| p.join("dev.gpumonitor.desktop/servers.json"))
        .ok_or_else(|| "Cannot locate the app configuration directory".into())
}

fn read_config() -> Result<Config, String> {
    match fs::read(config_path()?) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid app configuration: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(format!("Cannot read app configuration: {e}")),
    }
}

fn json<T: Serialize>(data: T) -> Result<(), String> {
    let report = Report {
        schema_version: 1,
        sampled_at: now(),
        data,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    );
    Ok(())
}

async fn query_server(
    config: ServerConfig,
    home: &Path,
    monitor: Monitor,
) -> Result<ServerView, String> {
    let mut view = ServerView::new(config)?;
    if monitor.gpu_enabled {
        match view.config.query_gpus(home, None).await {
            Ok(gpus) => {
                view.gpus = gpus;
                view.updated_at = Some(now());
                view.status = "online".into();
            }
            Err(error) => {
                view.status = "error".into();
                view.error = Some(error);
            }
        }
    } else {
        view.status = "paused".into();
    }
    if monitor.cpu_enabled {
        match view.config.query_cpu(home).await {
            Ok(cpu) => {
                view.cpu = Some(cpu);
                if !monitor.gpu_enabled {
                    view.status = "online".into();
                }
            }
            Err(error) => {
                view.cpu_error = Some(error);
                if !monitor.gpu_enabled {
                    view.status = "error".into();
                }
            }
        }
    }
    Ok(view)
}

async fn execute(options: Options) -> Result<i32, String> {
    if ["--help", "-h"].contains(&options.command.as_str()) {
        print!("{HELP}");
        return Ok(0);
    }
    if options.command == "--version" {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    let home = dirs::home_dir().ok_or("Cannot locate home directory")?;
    if options.command == "keys" {
        let connection = ssh::parse_connection("ssh localhost")?;
        let path = ssh::identity_path(&connection, &home);
        let exists = path.is_file();
        if options.json {
            json(
                serde_json::json!({ "path": path, "exists": exists, "publicExists": PathBuf::from(format!("{}.pub", path.display())).is_file() }),
            )?;
        } else {
            println!(
                "{}  {}",
                if exists { "FOUND" } else { "NOT_CREATED" },
                path.display()
            );
        }
        return Ok(0);
    }
    let config = read_config()?;
    if options.command == "settings" {
        json(serde_json::json!({
            "global": config.global,
            "overlay": config.overlay,
            "monitors": config.monitors,
            "gpuPolicies": config.policies
        }))?;
        return Ok(0);
    }
    if config.servers.len() > 32 {
        return Err("Configuration contains more than 32 servers".into());
    }
    if options.command == "servers" {
        if options.json {
            json(config.servers)?;
        } else {
            for server in config.servers {
                let target = match server.source {
                    Source::Local => "local",
                    Source::Ssh => &server.ssh_command,
                };
                println!("{}\t{}\t{}", server.id, server.name, target);
            }
        }
        return Ok(0);
    }
    let selected: Vec<_> = config
        .servers
        .into_iter()
        .filter(|s| {
            options
                .server
                .as_ref()
                .is_none_or(|id| s.id == *id || s.name == *id)
        })
        .collect();
    if selected.is_empty() {
        return Err("No matching servers. Add a server in the desktop app first; use 'servers --json' to list IDs.".into());
    }
    if options.server.is_some() && selected.len() > 1 {
        return Err("Server name is ambiguous; use its unique ID.".into());
    }
    if options.command == "history" {
        let mut archive = history::load(&config_path()?.with_file_name("history.json"), now())?;
        let mut data = Vec::new();
        for server in selected {
            let samples = archive.remove(&server.id).unwrap_or_default();
            if !options.json {
                println!(
                    "{} | {} resource histories | {} saved samples (up to 24h)",
                    server.name,
                    samples.len(),
                    samples.values().map(Vec::len).sum::<usize>()
                );
            }
            data.push(serde_json::json!({"id":server.id,"name":server.name,"history":samples}));
        }
        if options.json {
            json(data)?;
        }
        return Ok(0);
    }
    let mut jobs = tokio::task::JoinSet::new();
    for (order, server_config) in selected.into_iter().enumerate() {
        let home = home.clone();
        let mut monitor = server_config.monitor(&config.global.monitor, &config.monitors);
        if options.cpu || options.gpu {
            monitor.cpu_enabled = options.cpu;
            monitor.gpu_enabled = options.gpu;
        }
        jobs.spawn(async move {
            let view = query_server(server_config, &home, monitor).await?;
            Ok::<_, String>((order, view))
        });
    }
    let mut ordered = Vec::new();
    while let Some(result) = jobs.join_next().await {
        ordered.push(result.map_err(|e| e.to_string())??);
    }
    ordered.sort_by_key(|(order, _)| *order);
    let servers: Vec<_> = ordered.into_iter().map(|(_, server)| server).collect();
    let (status, exit) = summarize(servers);
    if options.json {
        json(status)?;
    } else {
        for server in status.servers {
            println!("{} [{}]", server.config.name, server.status);
            if let Some(error) = server.error {
                println!("  {error}");
            }
            if let Some(cpu) = server.cpu {
                println!(
                    "  CPU {} logical cores | {}% | RAM {:.0} / {:.0} MiB",
                    cpu.logical_cpus,
                    cpu.utilization
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "N/A".into()),
                    cpu.memory_used,
                    cpu.memory_total
                );
            }
            if let Some(error) = server.cpu_error {
                println!("  CPU error: {error}");
            }
            for gpu in server.gpus {
                let value =
                    |v: Option<f64>| v.map(|n| format!("{n:.0}")).unwrap_or_else(|| "N/A".into());
                println!(
                    "  GPU {} | {} | {}% | {} / {} MiB | {} C",
                    gpu.index,
                    gpu.name,
                    value(gpu.utilization),
                    value(gpu.memory_used),
                    value(gpu.memory_total),
                    value(gpu.temperature)
                );
            }
        }
    }
    Ok(exit)
}

pub fn run(args: Vec<String>) -> i32 {
    let json_requested = args.iter().any(|s| s == "--json");
    let result = parse(&args).and_then(|options| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        runtime.block_on(execute(options))
    });
    match result {
        Ok(code) => code,
        Err(error) => {
            if json_requested {
                println!(
                    "{}",
                    serde_json::json!({"schemaVersion": 1, "error": error})
                );
            } else {
                eprintln!("{error}");
            }
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_success_counts_online_without_hiding_collector_failures() {
        let mut server = ServerView::new(crate::ServerConfig {
            id: "test-server".into(),
            name: "Test server".into(),
            source: Source::Ssh,
            ssh_command: "ssh example.com".into(),
        })
        .unwrap();
        server.status = "error".into();
        server.error = Some("GPU query failed".into());
        server.cpu = Some(crate::cpu::Cpu {
            logical_cpus: 2,
            utilization: Some(20.0),
            memory_used: 128.0,
            memory_total: 1024.0,
            updated_at: 100,
        });
        let (status, exit) = summarize(vec![server.clone()]);
        assert_eq!(status.servers_online, 1);
        assert_eq!(status.gpus_online, 0);
        assert_eq!(exit, 2);
        assert_eq!(status.servers[0].error.as_deref(), Some("GPU query failed"));

        server.cpu_error = Some("CPU query failed".into());
        let (status, exit) = summarize(vec![server]);
        assert_eq!(status.servers_online, 0);
        assert_eq!(exit, 2);
    }

    #[test]
    fn local_cpu_request_returns_a_collector_error_without_ssh() {
        let config = ServerConfig {
            id: "local-test".into(),
            name: "Local test".into(),
            source: Source::Local,
            ssh_command: String::new(),
        };
        let monitor = Monitor {
            cpu_enabled: true,
            gpu_enabled: false,
            ..Monitor::default()
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let server = runtime
            .block_on(query_server(config, Path::new("unused-home"), monitor))
            .unwrap();
        assert_eq!(server.status, "error");
        assert!(server.error.is_none());
        assert!(server.cpu.is_none());
        assert!(server.cpu_error.is_some());
        let (status, exit) = summarize(vec![server]);
        assert_eq!(exit, 2);
        assert_eq!(status.servers_online, 0);
        let data = serde_json::to_value(status).unwrap();
        assert_eq!(data["servers"][0]["source"], "local");
    }

    #[test]
    fn parses_agent_commands_and_rejects_unknown_options() {
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse(&args(&["status", "--json", "--server", "lab"])).unwrap(),
            Options {
                command: "status".into(),
                json: true,
                server: Some("lab".into()),
                cpu: false,
                gpu: false,
            }
        );
        assert!(parse(&args(&["status", "--server"])).is_err());
        assert!(parse(&args(&["status", "--password", "secret"])).is_err());
        assert!(parse(&args(&["servers", "--server", "lab"])).is_err());
    }
}
