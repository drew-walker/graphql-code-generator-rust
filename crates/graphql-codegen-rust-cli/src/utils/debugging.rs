use std::time::{Duration, Instant};

pub fn debug_log(message: impl AsRef<str>) {
    eprintln!("{}", message.as_ref());
}

pub fn debug_log_if(enabled: bool, message: impl AsRef<str>) {
    if enabled {
        debug_log(message);
    }
}

pub fn debug_event(enabled: bool, message: impl AsRef<str>) {
    if enabled {
        eprintln!("[codegen:debug] {}", message.as_ref());
    }
}

pub fn timing_enabled_from_env() -> bool {
    std::env::var_os("CODEGEN_TIMING").is_some()
        || std::env::var_os("DEBUG").is_some()
        || std::env::var_os("VERBOSE").is_some()
}

pub fn timing_log(enabled: bool, label: impl AsRef<str>, duration: Duration) {
    if enabled {
        eprintln!(
            "[codegen:debug] {} took {}",
            label.as_ref(),
            format_duration(duration)
        );
    }
}

pub fn debug_timing(enabled: bool, label: impl AsRef<str>, started: Instant) {
    timing_log(enabled, label, started.elapsed());
}

pub fn format_duration(duration: Duration) -> String {
    let millis = duration.as_secs_f64() * 1000.0;
    if millis >= 1000.0 {
        format!("{:.3}s", millis / 1000.0)
    } else {
        format!("{millis:.1}ms")
    }
}
