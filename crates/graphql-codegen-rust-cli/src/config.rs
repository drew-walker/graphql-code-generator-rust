use anyhow::{Context as _, Result};
use clap::{ArgAction, Args};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::js_plugin_bridge::JsPluginHost;
use crate::utils::debugging::{debug_event, debug_timing};
use plugin_helpers::profiler::{Profiler, create_noop_profiler, create_profiler};
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::{Config, PluginContext, WatchValue};

#[derive(Debug, Clone, Default, Args)]
pub struct CliFlags {
    /// Path to the codegen YAML config file.
    #[arg(long, default_value = "codegen.yml")]
    pub config: String,

    /// Watch files and re-run on changes. Optionally provide one or more paths/globs.
    ///
    /// Mirrors the TS type: boolean | string | string[].
    /// - not provided => false
    /// - `--watch` (no value) => true
    /// - `--watch src/**/*.graphql` => string
    /// - `--watch a --watch b` (or repeated) => string[]
    #[arg(
        long,
        value_name = "PATH",
        num_args = 0..=1,
        action = ArgAction::Append,
        default_missing_value = "__WATCH_TRUE__"
    )]
    pub watch: Vec<String>,

    /// Modules to preload (Node's `--require` equivalent).
    #[arg(long, action = ArgAction::Append)]
    pub require: Vec<String>,

    /// Comma-separated plugin names to run natively in Rust (e.g. `--native-plugins=add,typescript`).
    ///
    /// Defaults to JS for all plugins; only listed plugins opt into native execution.
    #[arg(long, value_delimiter = ',', action = ArgAction::Append)]
    pub native_plugins: Vec<String>,

    /// Overwrite existing generated files.
    #[arg(long, action = ArgAction::SetTrue)]
    pub overwrite: bool,

    /// Project name / workspace selection (matches TS `project: string`).
    #[arg(long, default_value = "default")]
    pub project: String,

    /// Suppress all output.
    #[arg(long, action = ArgAction::SetTrue)]
    pub silent: bool,

    /// Only print errors.
    #[arg(long, action = ArgAction::SetTrue)]
    pub errors_only: bool,

    /// Enable performance profiling output.
    #[arg(long, action = ArgAction::SetTrue)]
    pub profile: bool,

    /// Check mode (do not write, exit non-zero if stale).
    #[arg(long, action = ArgAction::SetTrue)]
    pub check: bool,

    /// Verbose logging.
    #[arg(long, action = ArgAction::SetTrue)]
    pub verbose: bool,

    /// Debug logging.
    #[arg(long, action = ArgAction::SetTrue)]
    pub debug: bool,

    /// Do not error when no documents are found.
    #[arg(long, action = ArgAction::SetTrue)]
    pub ignore_no_documents: bool,

    /// Emit legacy CommonJS import style.
    #[arg(long, action = ArgAction::SetTrue)]
    pub emit_legacy_common_js_imports: bool,

    /// File extension to append to imports (e.g. ".js", ".mjs"). Use `--import-extension ""` for none.
    #[arg(long, value_name = "EXT")]
    pub import_extension: Option<String>,
}

#[derive(Debug)]
pub struct CodegenContext {
    /// Original config passed in (TS: `_config`).
    base_config: Option<Config>,
    /// Resolved, cached config (TS: `config`).
    resolved_config: Option<Config>,
    /// Which GraphQL Config project to use (stubbed for now).
    project: Option<String>,
    /// Check mode enabled.
    check_mode: bool,
    plugin_context: PluginContext,

    pub cwd: PathBuf,
    pub filepath: Option<PathBuf>,
    pub profiler: Profiler,
    pub profiler_output: Option<String>,
    pub check_mode_stale_files: Vec<String>,
    pub js_plugin_host: Option<JsPluginHost>,

    /// Keep CLI flags around so we can keep parity with TS' `updateContextWithCliFlags`.
    pub flags: CliFlags,
}

impl Clone for CodegenContext {
    fn clone(&self) -> Self {
        Self {
            base_config: self.base_config.clone(),
            resolved_config: self.resolved_config.clone(),
            project: self.project.clone(),
            check_mode: self.check_mode,
            plugin_context: self.plugin_context.clone(),
            cwd: self.cwd.clone(),
            filepath: self.filepath.clone(),
            profiler: create_noop_profiler(),
            profiler_output: self.profiler_output.clone(),
            check_mode_stale_files: self.check_mode_stale_files.clone(),
            js_plugin_host: None,
            flags: self.flags.clone(),
        }
    }
}

/// Options passed to `loadCodegenConfig`, mirroring `LoadCodegenConfigOptions`.
#[allow(dead_code)]
#[derive(Default)]
pub struct LoadCodegenConfigOptions {
    pub config_file_path: Option<PathBuf>,
    pub module_name: Option<String>,
    pub search_places: Option<Vec<String>>,
    pub package_prop: Option<String>,
}

/// Return value of `loadCodegenConfig`, mirroring `LoadCodegenConfigResult`.
pub struct LoadCodegenConfigResult {
    pub filepath: PathBuf,
    pub config: Config,
    pub is_empty: bool,
}

/// Mirrors TS `createContext`.
pub async fn create_context(flags: CliFlags) -> Result<CodegenContext> {
    // TS: handle cliFlags.require.
    //
    // Upstream loads dotenv via `--require dotenv/config` (see repo root package.json scripts).
    // In Rust we don't execute Node requires, but we can mirror the behavior for this specific
    // module name by loading `.env` before evaluating config files.
    maybe_load_dotenv_from_require(&flags);

    let custom_config_path = get_custom_config_path(&flags);
    let mut context = load_context(custom_config_path).await?;
    context.flags = flags;
    update_context_with_cli_flags(&mut context);
    Ok(context)
}

fn maybe_load_dotenv_from_require(flags: &CliFlags) {
    let wants_dotenv = flags
        .require
        .iter()
        .any(|m| m == "dotenv/config" || m == "dotenv/config.js");
    if !wants_dotenv {
        return;
    }

    // Upstream's `dotenv/config` supports both `DOTENV_CONFIG_PATH` and `dotenv_config_path`.
    // If neither is provided, it defaults to `.env` in the current working directory.
    let raw_path = std::env::var("DOTENV_CONFIG_PATH")
        .ok()
        .or_else(|| std::env::var("dotenv_config_path").ok());

    if let Some(p) = raw_path
        && !p.is_empty()
    {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let candidate = PathBuf::from(&p);

        // 1) Try as-is (dotenv/config behavior: path resolved relative to cwd).
        let mut tried: Vec<PathBuf> = vec![candidate.clone()];

        // 2) If it doesn't exist, also try relative to the config file's directory (common in scripts).
        // This is a robustness improvement when running from subdirectories.
        let config_path = cwd.join(&flags.config);
        if let Some(parent) = config_path.parent() {
            tried.push(parent.join(&candidate));
        }

        for t in tried {
            if std::fs::metadata(&t).is_ok() {
                let _ = dotenvy::from_path(&t);
                break;
            }
        }
        return;
    }

    let _ = dotenvy::dotenv();
}

/// Mirrors TS `loadContext`.
pub async fn load_context(config_file_path: Option<PathBuf>) -> Result<CodegenContext> {
    // findAndLoadGraphQLConfig is stubbed — fall straight through to loadCodegenConfig.

    let result = load_codegen_config(LoadCodegenConfigOptions {
        config_file_path: config_file_path.clone(),
        ..Default::default()
    })
    .await?;

    let result = match result {
        None => {
            if let Some(path) = &config_file_path {
                anyhow::bail!(
                    r#"
        Config {} does not exist.

          $ graphql-codegen --config {}

        Please make sure the --config points to a correct file.
      "#,
                    path.display(),
                    path.display()
                );
            }
            anyhow::bail!(
                r#"Unable to find Codegen config file! \n
        Please make sure that you have a configuration file under the current directory!
      "#
            );
        }
        Some(r) => r,
    };

    if result.is_empty {
        anyhow::bail!(
            r#"Found Codegen config file but it was empty! \n
        Please make sure that you have a valid configuration file under the current directory!
      "#
        );
    }

    let cwd = result
        .config
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    Ok(CodegenContext::new(
        Some(result.config),
        Some(cwd),
        Some(result.filepath),
        CliFlags::default(),
    ))
}

/// Mirrors TS `loadCodegenConfig`.
pub async fn load_codegen_config(
    options: LoadCodegenConfigOptions,
) -> Result<Option<LoadCodegenConfigResult>> {
    let mut config_file_path = options
        .config_file_path
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let is_dir = std::fs::metadata(&config_file_path)
        .map(|m| m.is_dir())
        .unwrap_or(false);

    if is_dir {
        // Mirrors `cosmiconfig(...).search()` for `codegen`.
        let search_places = generate_search_places("codegen");
        for filename in search_places {
            let candidate = config_file_path.join(&filename);
            if std::fs::metadata(&candidate).is_ok() {
                config_file_path = candidate;
                break;
            }
        }
        // If we didn't find anything, align with upstream "no result".
        if std::fs::metadata(&config_file_path)
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            return Ok(None);
        }
    }

    let ext = config_file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let config = match ext {
        "json" => load_json_config(&config_file_path)?,
        "yaml" | "yml" => load_yaml_config(&config_file_path)?,
        "js" | "ts" | "mts" | "cts" => load_js_config(&config_file_path).await?,
        _ => load_yaml_config(&config_file_path)?,
    };

    Ok(Some(LoadCodegenConfigResult {
        filepath: config_file_path,
        config,
        is_empty: false,
    }))
}

fn load_json_config(filepath: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(filepath)?;
    serde_json::from_str(&content).map_err(Into::into)
}

fn load_yaml_config(filepath: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(filepath)?;
    serde_yaml::from_str(&content).map_err(Into::into)
}

/// Mirrors the TS `customLoader('ts')` which uses `jiti` to execute the file and return its
/// default export. Here we shell out to `node --experimental-strip-types` to do the same.
async fn load_js_config(filepath: &Path) -> Result<Config> {
    let abs = filepath
        .canonicalize()
        .with_context(|| format!("Config file not found: {}", filepath.display()))?;
    let abs_str = abs.to_string_lossy();

    let script = format!(
        "import(String.raw`file://{abs_str}`)\
         .then(m => {{ process.stdout.write(JSON.stringify(m.default ?? m)); process.exit(0); }})\
         .catch(e => {{ process.stderr.write(String(e)); process.exit(1); }});"
    );

    let output = tokio::process::Command::new("node")
        .args([
            "--experimental-strip-types",
            "--input-type=module",
            "-e",
            &script,
        ])
        .output()
        .await
        .context("Failed to spawn `node` — is it installed and in PATH?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to load config file {}: {}",
            abs.display(),
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse config JSON from {}: {e}\nJSON was: {stdout}",
            abs.display()
        )
    })
}

/// Mirrors TS `getCustomConfigPath`.
fn get_custom_config_path(cli_flags: &CliFlags) -> Option<PathBuf> {
    let config_file = &cli_flags.config;
    if config_file.is_empty() {
        return None;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    Some(cwd.join(config_file))
}

/// Mirrors TS `ensureContext` (for now, "context only" is fine).
pub fn ensure_context(input: CodegenContext) -> CodegenContext {
    input
}

impl CodegenContext {
    pub fn new(
        base_config: Option<Config>,
        cwd: Option<PathBuf>,
        filepath: Option<PathBuf>,
        flags: CliFlags,
    ) -> Self {
        let cwd =
            cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        Self {
            base_config,
            resolved_config: None,
            project: None,
            check_mode: false,
            plugin_context: PluginContext::default(),

            cwd,
            filepath,
            profiler: create_noop_profiler(),
            profiler_output: None,
            check_mode_stale_files: vec![],
            js_plugin_host: None,
            flags,
        }
    }

    pub fn use_project(&mut self, name: Option<String>) {
        self.project = name;
        // GraphQLConfig support is stubbed, so this doesn't change resolution yet.
        self.resolved_config = None;
    }

    pub fn enable_check_mode(&mut self) {
        self.check_mode = true;
    }

    pub fn check_mode(&self) -> bool {
        self.check_mode
    }

    pub fn use_profiler(&mut self) {
        self.profiler = create_profiler();
        self.profiler_output = Some(default_profiler_output_name());
    }

    pub async fn load_schema_with_timing(
        &self,
        pointers: &[String],
        timing_enabled: bool,
    ) -> Result<SchemaGenerationInput> {
        graphql_tools_load::load_schema_with_timing(&self.cwd, pointers, timing_enabled).await
    }

    pub async fn js_plugin_host(&mut self, timing_enabled: bool) -> Result<&mut JsPluginHost> {
        if self.js_plugin_host.is_none() {
            let started = Instant::now();
            debug_event(
                timing_enabled,
                format!(
                    "starting persistent JS plugin host in {}",
                    self.cwd.display()
                ),
            );
            self.js_plugin_host = Some(JsPluginHost::spawn(&self.cwd).await?);
            debug_timing(timing_enabled, "spawn persistent JS plugin host", started);
        }
        Ok(self
            .js_plugin_host
            .as_mut()
            .expect("js plugin host initialized"))
    }

    pub fn get_config(&mut self) -> Config {
        if let Some(cfg) = &self.resolved_config {
            return cfg.clone();
        }

        // TS behavior:
        // - If GraphQLConfig exists: resolve project extension('codegen') + schema/documents + pluginContext
        // - Else: merge base config + pluginContext, and ensure cwd is present.
        //
        // We stub GraphQLConfig resolution. For now:
        // - take `base_config` or default
        // - apply plugin_context
        let mut cfg = self.base_config.clone().unwrap_or_default();
        cfg.plugin_context = self.plugin_context.clone();

        self.resolved_config = Some(cfg.clone());
        cfg
    }

    pub fn update_config(&mut self, patch: PartialConfig) {
        let mut current = self.get_config();
        patch.apply_to(&mut current);
        self.resolved_config = Some(current);
    }
}

fn default_profiler_output_name() -> String {
    // TS: codegen-YYYYMMDDTHHMMSS.json (normalized from ISO).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("codegen-{now}.json")
}

#[allow(dead_code)] // wired up when `load_codegen_config` is ported
pub fn generate_search_places(module_name: &str) -> Vec<String> {
    let extensions = ["json", "yaml", "yml", "js", "ts", "config.js"];
    let mut regular: Vec<String> = extensions
        .iter()
        .map(|ext| format!("{module_name}.{ext}"))
        .collect();
    let mut dot: Vec<String> = extensions
        .iter()
        .filter(|ext| **ext != "config.js")
        .map(|ext| format!(".{module_name}rc.{ext}"))
        .collect();
    regular.append(&mut dot);
    regular.push("package.json".to_string());
    regular
}

/// Mirrors TS `updateContextWithCliFlags`.
pub fn update_context_with_cli_flags(context: &mut CodegenContext) {
    let flags = context.flags.clone();

    let mut patch = PartialConfig {
        config_file_path: context
            .filepath
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        ..Default::default()
    };

    // watch
    if !flags.watch.is_empty() {
        // See `CliFlags.watch` docstring for mapping rules.
        if flags.watch.len() == 1 && flags.watch[0] == "__WATCH_TRUE__" {
            patch.watch = Some(WatchValue::Bool(true));
        } else if flags.watch.len() == 1 {
            patch.watch = Some(WatchValue::String(flags.watch[0].clone()));
        } else {
            patch.watch = Some(WatchValue::Strings(flags.watch.clone()));
        }
    }

    if flags.overwrite {
        patch.overwrite = Some(true);
    }

    if flags.silent {
        patch.silent = Some(true);
    }

    if flags.verbose || std::env::var_os("VERBOSE").is_some() {
        patch.verbose = Some(true);
    }

    if flags.debug || std::env::var_os("DEBUG").is_some() {
        patch.debug = Some(true);
    }

    if flags.errors_only {
        patch.errors_only = Some(true);
    }

    if flags.ignore_no_documents {
        patch.ignore_no_documents = Some(true);
    }

    if flags.emit_legacy_common_js_imports {
        patch.emit_legacy_common_js_imports = Some(true);
    }

    if let Some(ext) = flags.import_extension.clone() {
        patch.import_extension = Some(ext);
    }

    if !flags.project.is_empty() {
        context.use_project(Some(flags.project));
    }

    if flags.profile {
        context.use_profiler();
    }

    if flags.check {
        context.enable_check_mode();
    }

    context.update_config(patch);
}

/// A small "partial Types.Config" patch object, mirroring TS' `Partial<Types.Config & { configFilePath?: string }>`
#[derive(Debug, Clone, Default)]
pub struct PartialConfig {
    pub watch: Option<WatchValue>,
    pub overwrite: Option<bool>,
    pub silent: Option<bool>,
    pub errors_only: Option<bool>,
    pub verbose: Option<bool>,
    pub debug: Option<bool>,
    pub ignore_no_documents: Option<bool>,
    pub emit_legacy_common_js_imports: Option<bool>,
    pub import_extension: Option<String>,
    pub config_file_path: Option<String>,
}

impl PartialConfig {
    pub fn apply_to(self, cfg: &mut Config) {
        if let Some(v) = self.watch {
            cfg.watch = v;
        }
        if let Some(v) = self.overwrite {
            cfg.overwrite = Some(v);
        }
        if let Some(v) = self.silent {
            cfg.silent = Some(v);
        }
        if let Some(v) = self.errors_only {
            cfg.errors_only = Some(v);
        }
        if let Some(v) = self.verbose {
            cfg.verbose = Some(v);
        }
        if let Some(v) = self.debug {
            cfg.debug = Some(v);
        }
        if let Some(v) = self.ignore_no_documents {
            cfg.ignore_no_documents = Some(v);
        }
        if let Some(v) = self.emit_legacy_common_js_imports {
            cfg.emit_legacy_common_js_imports = Some(v);
        }
        if let Some(v) = self.import_extension {
            cfg.import_extension = Some(v);
        }
        if let Some(v) = self.config_file_path {
            cfg.config_file_path = Some(v);
        }
    }
}
