use anyhow::Result;
use clap::{ArgAction, Args};
use std::path::PathBuf;

use plugin_helpers::profiler::{Profiler, create_noop_profiler, create_profiler};
use plugin_helpers::types::{Config, PluginContext};

#[derive(Debug, Clone, Args)]
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

#[derive(Debug, Clone)]
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

    /// Keep CLI flags around so we can keep parity with TS' `updateContextWithCliFlags`.
    pub flags: CliFlags,
}

pub fn create_context(flags: CliFlags) -> Result<CodegenContext> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut ctx = CodegenContext::new(base_config_from_flags(&flags), Some(cwd), None, flags);
    update_context_with_cli_flags(&mut ctx);
    Ok(ctx)
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

    // pub fn get_plugin_context(&self) -> &PluginContext {
    //     &self.plugin_context
    // }

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
    // We'll approximate without pulling in chrono.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("codegen-{now}.json")
}

fn base_config_from_flags(_flags: &CliFlags) -> Option<Config> {
    // TODO: actually load config file and parse YAML, etc. (stubbed)
    Some(Config::default())
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
        patch.watch = Some(true);
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
    pub watch: Option<bool>,
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
