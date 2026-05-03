use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct Profiler {
    enabled: bool,
    events: Arc<Mutex<Vec<ProfileEvent>>>,
}

#[derive(Debug, Clone)]
struct ProfileEvent {
    label: String,
    duration: Duration,
}

#[derive(Debug)]
pub struct Profiled<T> {
    pub result: T,
    pub error: Option<anyhow::Error>,
}

impl Profiler {
    pub async fn run<T, Fut>(&self, f: impl FnOnce() -> Fut, label: &str) -> Result<T>
    where
        Fut: std::future::Future<Output = Result<T>>,
    {
        if !self.enabled {
            return f().await;
        }

        let started = Instant::now();
        let result = f().await;
        self.record(label, started.elapsed());
        result
    }

    pub async fn run_profiled<T, Fut>(
        &self,
        f: impl FnOnce() -> Fut,
        label: &str,
        on_error_result: T,
    ) -> Profiled<T>
    where
        Fut: std::future::Future<Output = Result<T>>,
    {
        let started = Instant::now();
        let result = f().await;
        if self.enabled {
            self.record(label, started.elapsed());
        }

        match result {
            Ok(result) => Profiled {
                result,
                error: None,
            },
            Err(error) => Profiled {
                result: on_error_result,
                error: Some(error),
            },
        }
    }

    pub fn collect(&self) -> serde_json::Value {
        let events = self
            .events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default();
        serde_json::json!({
            "events": events
                .into_iter()
                .map(|event| serde_json::json!({
                    "label": event.label,
                    "durationMs": event.duration.as_secs_f64() * 1000.0,
                }))
                .collect::<Vec<_>>()
        })
    }

    fn record(&self, label: &str, duration: Duration) {
        if let Ok(mut events) = self.events.lock() {
            events.push(ProfileEvent {
                label: label.to_string(),
                duration,
            });
        }
        eprintln!(
            "[codegen:profile] {label} took {}",
            format_duration(duration)
        );
    }
}

pub fn create_noop_profiler() -> Profiler {
    Profiler::default()
}

pub fn create_profiler() -> Profiler {
    Profiler {
        enabled: true,
        events: Arc::new(Mutex::new(Vec::new())),
    }
}

fn format_duration(duration: Duration) -> String {
    let millis = duration.as_secs_f64() * 1000.0;
    if millis >= 1000.0 {
        format!("{:.3}s", millis / 1000.0)
    } else {
        format!("{millis:.1}ms")
    }
}
