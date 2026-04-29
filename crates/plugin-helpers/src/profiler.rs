use anyhow::Result;

#[derive(Debug, Clone, Default)]
pub struct Profiler;

#[derive(Debug)]
pub struct Profiled<T> {
    pub result: T,
    pub error: Option<anyhow::Error>,
}

impl Profiler {
    pub async fn run<T, Fut>(&self, f: impl FnOnce() -> Fut, _label: &str) -> Result<T>
    where
        Fut: std::future::Future<Output = Result<T>>,
    {
        f().await
    }

    pub async fn run_profiled<T, Fut>(
        &self,
        f: impl FnOnce() -> Fut,
        _label: &str,
        on_error_result: T,
    ) -> Profiled<T>
    where
        Fut: std::future::Future<Output = Result<T>>,
    {
        match f().await {
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
        serde_json::json!({})
    }
}

pub fn create_noop_profiler() -> Profiler {
    Profiler
}

pub fn create_profiler() -> Profiler {
    // TODO: real profiler; noop for now.
    Profiler
}
