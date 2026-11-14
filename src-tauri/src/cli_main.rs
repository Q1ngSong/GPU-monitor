fn main() {
    if std::env::var_os(gpu_monitor_lib::askpass::NO_PROMPT_ENV).is_some() {
        std::process::exit(1);
    }
    std::process::exit(gpu_monitor_lib::cli::run(
        std::env::args().skip(1).collect(),
    ));
}
