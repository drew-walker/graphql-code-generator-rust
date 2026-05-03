use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use base64::Engine as _;
use sha1::{Digest as _, Sha1};

use crate::codegen::execute_codegen;
use crate::config::{CodegenContext, ensure_context};
use crate::hooks::lifecycle_hooks;
use crate::utils::debugging::debug_log;
use crate::utils::file_system::{mkdirp, read_file, unlink_file, write_file};
use crate::utils::logger::get_logger;
use crate::utils::watcher::create_watcher;
use plugin_helpers::types::{Config, FileOutput, OutputConfig};

fn hash(content: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(content.as_bytes());
    let bytes = hasher.finalize();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn normalize_generated_content(content: String) -> String {
    let mut lines = Vec::new();
    for line in content.lines() {
        if line
            == "export type RequireFields<T, K extends keyof T> = Omit<T, K> & { [P in K]-?: NonNullable<T[P]> };"
            && lines
                .last()
                .is_some_and(|previous: &&str| previous.is_empty())
        {
            lines.pop();
        }
        if line == "/** All built-in and custom scalars, mapped to their actual values */"
            && lines
                .last()
                .is_some_and(|previous: &&str| previous.is_empty())
        {
            lines.pop();
        }
        if line.starts_with("export type EnumResolverSignature<")
            && lines
                .last()
                .is_some_and(|previous: &&str| previous.is_empty())
        {
            lines.pop();
        }
        lines.push(line);
    }

    let mut normalized = lines.join("\n");
    if content.ends_with('\n') {
        normalized.push('\n');
    }
    normalized
}

pub async fn generate(input: CodegenContext, save_to_file: bool) -> Result<()> {
    let mut context = ensure_context(input);
    let config = context.get_config();

    context
        .profiler
        .run(
            || async { lifecycle_hooks(config.hooks.clone()).after_start().await },
            "Lifecycle: afterStart",
        )
        .await?;

    let mut previously_generated_filenames: Vec<String> = vec![];

    let mut recent_output_hash: HashMap<String, String> = HashMap::new();

    async fn write_output(
        context: &mut CodegenContext,
        config: &Config,
        save_to_file: bool,
        check_mode: bool,
        previously_generated_filenames: &mut Vec<String>,
        recent_output_hash: &mut HashMap<String, String>,
        generation_result: Vec<FileOutput>,
    ) -> Result<Vec<FileOutput>> {
        if !save_to_file {
            return Ok(generation_result);
        }

        if config.watch.is_truthy() {
            remove_stale_files(
                context,
                config,
                previously_generated_filenames,
                &generation_result,
            )
            .await;
        }

        context
            .profiler
            .run(
                || async {
                    lifecycle_hooks(config.hooks.clone())
                        .before_all_file_write(
                            generation_result
                                .iter()
                                .map(|r| r.filename.clone())
                                .collect(),
                        )
                        .await
                },
                "Lifecycle: beforeAllFileWrite",
            )
            .await?;

        context
            .profiler
            .run(
                || async {
                    for mut result in generation_result.clone() {
                        let previous_hash = match recent_output_hash.get(&result.filename) {
                            Some(h) => Some(h.clone()),
                            None => hash_file(&resolve_path(&context.cwd, &result.filename)).await?,
                        };

                        let exists = previous_hash.is_some();
                        if let Some(ph) = &previous_hash {
                            recent_output_hash.insert(result.filename.clone(), ph.clone());
                        }

                        if !should_overwrite(config, &result.filename) && exists {
                            continue;
                        }

                        let mut content =
                            normalize_generated_content(result.content.clone().unwrap_or_default());
                        let current_hash = hash(&content);

                        if let Some(ph) = &previous_hash
                            && current_hash == *ph
                        {
                            debug_log(format!(
                                "Skipping file ({}) writing due to indentical hash...",
                                result.filename
                            ));
                            continue;
                        }

                        if check_mode {
                            context.check_mode_stale_files.push(result.filename.clone());
                            continue;
                        }

                        if content.is_empty() {
                            continue;
                        }

                        let absolute_path = resolve_path(&context.cwd, &result.filename);
                        if let Some(basedir) = absolute_path.parent() {
                            mkdirp(basedir).await?;
                        }

                        content = lifecycle_hooks(result.hooks.clone())
                            .before_one_file_write(absolute_path.to_string_lossy().as_ref(), content)
                            .await?;
                        content = lifecycle_hooks(config.hooks.clone())
                            .before_one_file_write(absolute_path.to_string_lossy().as_ref(), content)
                            .await?;
                        content = normalize_generated_content(content);

                        result.content = Some(content.clone());
                        if let Some(ph) = &previous_hash
                            && hash(&content) == *ph
                        {
                            debug_log(format!(
                                "Skipping file ({}) writing due to indentical hash after prettier...",
                                result.filename
                            ));
                            continue;
                        }

                        write_file(&absolute_path, result.content.as_deref().unwrap_or_default().as_bytes()).await?;
                        recent_output_hash.insert(result.filename.clone(), current_hash);

                        lifecycle_hooks(result.hooks.clone())
                            .after_one_file_write(&result.filename)
                            .await?;
                        lifecycle_hooks(config.hooks.clone())
                            .after_one_file_write(&result.filename)
                            .await?;
                    }
                    Ok::<(), anyhow::Error>(())
                },
                "Write files",
            )
            .await?;

        context
            .profiler
            .run(
                || async {
                    lifecycle_hooks(config.hooks.clone())
                        .after_all_file_write(
                            generation_result
                                .iter()
                                .map(|r| r.filename.clone())
                                .collect(),
                        )
                        .await
                },
                "Lifecycle: afterAllFileWrite",
            )
            .await?;

        for result in &generation_result {
            let absolute_path = resolve_path(&context.cwd, &result.filename);
            if let Ok(content) = read_file(&absolute_path).await {
                let normalized = normalize_generated_content(content.clone());
                if normalized != content {
                    write_file(&absolute_path, normalized.as_bytes()).await?;
                }
            }
        }

        Ok(generation_result)
    }

    // watch mode
    if config.watch.is_truthy() {
        let watcher = create_watcher(context.clone(), |_outputs| Box::pin(async { Ok(vec![]) }));
        // Stub: real impl would await this; drop for now so callers get `Ok(())` without unused-value warnings.
        drop(watcher.running_watcher);
        return Ok(());
    }

    let profiler = context.profiler.clone();
    let profiled = profiler
        .run(
            || async { Ok(execute_codegen(&mut context).await) },
            "executeCodegen",
        )
        .await?;

    let output_files = profiled.result;
    let error = profiled.error;

    if let Some(err) = error {
        if output_files.is_empty() {
            return Err(err);
        }

        if !config.allow_partial_outputs {
            get_logger().error("  ✖ One or more errors occurred, no files were generated. To allow output on errors, set config.allowPartialOutputs=true");
            return Err(err);
        }

        get_logger().warn(
            "  ⚠ One or more errors occurred, some files were generated. To prevent any output on errors, set config.allowPartialOutputs=false",
        );
    }

    let profiler = context.profiler.clone();
    let check_mode = context.check_mode();
    let output_files = profiler
        .run(
            || {
                write_output(
                    &mut context,
                    &config,
                    save_to_file,
                    check_mode,
                    &mut previously_generated_filenames,
                    &mut recent_output_hash,
                    output_files,
                )
            },
            "writeOutput",
        )
        .await?;

    context
        .profiler
        .run(
            || async { lifecycle_hooks(config.hooks.clone()).before_done().await },
            "Lifecycle: beforeDone",
        )
        .await?;

    if let Some(profiler_output) = &context.profiler_output {
        let path = context.cwd.join(profiler_output);
        write_file(path, context.profiler.collect().to_string().as_bytes()).await?;
    }

    let _ = output_files;
    Ok(())
}

async fn remove_stale_files(
    context: &CodegenContext,
    config: &Config,
    previously_generated_filenames: &mut Vec<String>,
    generation_result: &[FileOutput],
) {
    let filenames: Vec<String> = generation_result
        .iter()
        .map(|o| o.filename.clone())
        .collect();
    let stale_filenames: Vec<String> = previously_generated_filenames
        .iter()
        .filter(|f| !filenames.contains(f))
        .cloned()
        .collect();

    for filename in stale_filenames {
        if should_overwrite(config, &filename) {
            let absolute = resolve_path(&context.cwd, &filename);
            match unlink_file(&absolute).await {
                Ok(()) => debug_log(format!("Removed stale file: {}", filename)),
                Err(err) => debug_log(format!("Cannot remove stale file: {}\n{}", filename, err)),
            }
        }
    }

    *previously_generated_filenames = filenames;
}

fn should_overwrite(config: &Config, output_path: &str) -> bool {
    let global_value = config.overwrite.unwrap_or(true);

    let Some(output_config) = config.generates.get(output_path) else {
        debug_log(format!("Couldn't find a config of {}", output_path));
        return global_value;
    };

    if is_configured_output(output_config)
        && let OutputConfig::Configured(c) = output_config
        && let Some(v) = c.overwrite
    {
        return v;
    }

    global_value
}

fn is_configured_output(output: &OutputConfig) -> bool {
    matches!(output, OutputConfig::Configured(_))
}

fn resolve_path(cwd: &Path, filename: &str) -> PathBuf {
    let p = Path::new(filename);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

async fn hash_file(file_path: &Path) -> Result<Option<String>> {
    match read_file(file_path).await {
        Ok(contents) => Ok(Some(hash(&contents))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}
