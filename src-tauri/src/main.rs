#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::var_os(gpu_monitor_lib::askpass::NO_PROMPT_ENV).is_some() {
        std::process::exit(1);
    }
    if std::env::var_os("GPU_MONITOR_ASKPASS_ADDR").is_some() {
        std::process::exit(gpu_monitor_lib::askpass::respond());
    }
    gpu_monitor_lib::run();
}
