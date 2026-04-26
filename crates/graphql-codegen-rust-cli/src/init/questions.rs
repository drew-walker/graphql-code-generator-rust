use anyhow::Context;
use colored::Colorize;
use inquire::{Confirm, MultiSelect, Select, Text, error::InquireError};

use super::plugins::get_plugin_choices;
use super::types::{InitAnswers, PluginOption, PossibleTargets, Tag};

// enum AppTypeKey {
//     Backend,
//     Angular,
//     React,
//     Stencil,
//     Vue,
//     GraphqlRequest,
//     Client
// }

struct AppTypeChoice {
    label: &'static str,
    // key: AppTypeKey,
    tags: Vec<Tag>,
    checked: bool,
}

/// Prompt the user and build `InitAnswers` (TS `getAnswers`).
pub async fn get_answers(possible_targets: &PossibleTargets) -> anyhow::Result<InitAnswers> {
    let p = possible_targets.clone();
    tokio::task::spawn_blocking(move || get_answers_blocking(&p))
        .await
        .context("join prompt task")?
}

fn get_answers_blocking(possible_targets: &PossibleTargets) -> anyhow::Result<InitAnswers> {
    let target_choices = get_application_type_choices(possible_targets);
    let labels: Vec<String> = target_choices.iter().map(|c| c.label.to_string()).collect();
    let default_idx = target_choices.iter().position(|c| c.checked).unwrap_or(0);

    let chosen_label = prompt_or_exit(
        Select::new("What type of application are you building?", labels.clone())
            .with_starting_cursor(default_idx)
            .prompt(),
    )?;
    let idx = labels
        .iter()
        .position(|l| l == &chosen_label)
        .context("selected application type not found in choices")?;
    let targets = target_choices[idx].tags.clone();

    let schema: String = prompt_or_exit(
        Text::new(&format!(
            "Where is your schema?: {}",
            "(path or url)".bright_black()
        ))
        .with_default("http://localhost:4000")
        .with_validator(|s: &str| {
            if s.is_empty() {
                Ok(inquire::validator::Validation::Invalid(
                    "must not be empty".into(),
                ))
            } else {
                Ok(inquire::validator::Validation::Valid)
            }
        })
        .prompt(),
    )?;

    let documents = if targets.contains(&Tag::Client)
        || targets.contains(&Tag::Angular)
        || targets.contains(&Tag::Stencil)
    {
        let default = get_documents_default_value(&targets);
        Some(prompt_or_exit(
            Text::new("Where are your operations and fragments?:")
                .with_default(&default)
                .with_validator(|s: &str| {
                    if s.is_empty() {
                        Ok(inquire::validator::Validation::Invalid(
                            "must not be empty".into(),
                        ))
                    } else {
                        Ok(inquire::validator::Validation::Valid)
                    }
                })
                .prompt(),
        )?)
    } else {
        None
    };

    let plugins = if !targets.contains(&Tag::Client) {
        let rows = get_plugin_choices(&targets);
        if rows.is_empty() {
            None
        } else {
            let labels: Vec<String> = rows.iter().map(|(l, _, _)| l.clone()).collect();
            // `inquire` expects default selections as indices (not a bool-per-row bitmap).
            let default_indices: Vec<usize> = rows
                .iter()
                .enumerate()
                .filter_map(|(idx, (_, _, checked))| checked.then_some(idx))
                .collect();
            let picked_labels = prompt_or_exit(
                MultiSelect::new("Pick plugins:", labels.clone())
                    .with_default(&default_indices)
                    .prompt(),
            )?;
            if picked_labels.is_empty() {
                return Err(anyhow::anyhow!("pick at least one plugin"));
            }
            let mut out = Vec::new();
            for chosen in picked_labels {
                let idx = labels
                    .iter()
                    .position(|l| l == &chosen)
                    .context("selected plugin not found in choices")?;
                out.push(rows[idx].1.clone());
            }
            Some(out)
        }
    } else {
        None
    };

    let plugin_slice = plugins.as_deref().unwrap_or(&[]);
    let output_default = get_output_default_value(&targets, plugin_slice);
    let output: String = prompt_or_exit(
        Text::new("Where to write the output:")
            .with_default(&output_default)
            .with_validator(|s: &str| {
                if s.is_empty() {
                    Ok(inquire::validator::Validation::Invalid(
                        "must not be empty".into(),
                    ))
                } else {
                    Ok(inquire::validator::Validation::Valid)
                }
            })
            .prompt(),
    )?;

    let introspection = prompt_or_exit(
        Confirm::new("Do you want to generate an introspection file?")
            .with_default(false)
            .prompt(),
    )?;

    let default_config = if targets.contains(&Tag::Client)
        || targets.contains(&Tag::TypeScript)
        || targets.contains(&Tag::Angular)
    {
        "codegen.ts"
    } else {
        "codegen.yml"
    };

    let config: String = prompt_or_exit(
        Text::new("How to name the config file?")
            .with_default(default_config)
            .with_validator(|s: &str| match config_filename_ok(s) {
                Ok(()) => Ok(inquire::validator::Validation::Valid),
                Err(msg) => Ok(inquire::validator::Validation::Invalid(msg.into())),
            })
            .prompt(),
    )?;

    let script: String = prompt_or_exit(
        Text::new("What script in package.json should run the codegen?")
            .with_default("codegen")
            .with_validator(|s: &str| {
                if s.is_empty() {
                    Ok(inquire::validator::Validation::Invalid(
                        "must not be empty".into(),
                    ))
                } else {
                    Ok(inquire::validator::Validation::Valid)
                }
            })
            .prompt(),
    )?;

    Ok(InitAnswers {
        targets,
        schema,
        documents,
        plugins,
        output,
        introspection,
        config,
        script,
    })
}

fn get_application_type_choices(possible_targets: &PossibleTargets) -> Vec<AppTypeChoice> {
    let with_flow_or_typescript = |mut tags: Vec<Tag>| {
        if possible_targets.typescript {
            tags.push(Tag::TypeScript);
        } else if possible_targets.flow {
            tags.push(Tag::Flow);
        } else if possible_targets.node {
            tags.push(Tag::TypeScript);
            tags.push(Tag::Flow);
        }
        tags
    };

    vec![
        AppTypeChoice {
            label: "Backend - API or server",
            // key: AppTypeKey::Backend,
            tags: with_flow_or_typescript(vec![Tag::Node]),
            checked: possible_targets.node,
        },
        AppTypeChoice {
            label: "Application built with Angular",
            // key: AppTypeKey::Angular,
            tags: vec![Tag::Angular],
            checked: possible_targets.angular,
        },
        AppTypeChoice {
            label: "Application built with React",
            // key: AppTypeKey::React,
            tags: with_flow_or_typescript(vec![Tag::React, Tag::Client]),
            checked: possible_targets.react,
        },
        AppTypeChoice {
            label: "Application built with Stencil",
            // key: AppTypeKey::Stencil,
            tags: vec![Tag::Stencil, Tag::TypeScript],
            checked: possible_targets.stencil,
        },
        AppTypeChoice {
            label: "Application built with Vue",
            // key: AppTypeKey::Vue,
            tags: with_flow_or_typescript(vec![Tag::Vue, Tag::Client]),
            checked: possible_targets.vue,
        },
        AppTypeChoice {
            label: "Application using graphql-request",
            //key: AppTypeKey::GraphqlRequest,
            tags: with_flow_or_typescript(vec![Tag::GraphqlRequest, Tag::Client]),
            checked: possible_targets.graphql_request,
        },
        AppTypeChoice {
            label: "Application built with other framework or vanilla JS",
            // key: AppTypeKey::Client,
            tags: vec![Tag::TypeScript, Tag::Flow],
            checked: possible_targets.browser
                && !possible_targets.angular
                && !possible_targets.react
                && !possible_targets.stencil,
        },
    ]
}

fn get_output_default_value(targets: &[Tag], plugins: &[PluginOption]) -> String {
    if targets.contains(&Tag::Client) {
        return "src/gql/".to_string();
    }
    if plugins.iter().any(|p| p.default_extension == ".tsx") {
        return "src/generated/graphql.tsx".to_string();
    }
    if plugins.iter().any(|p| p.default_extension == ".ts") {
        return "src/generated/graphql.ts".to_string();
    }
    "src/generated/graphql.js".to_string()
}

fn get_documents_default_value(targets: &[Tag]) -> String {
    if targets.contains(&Tag::Vue) {
        return "src/**/*.vue".to_string();
    }
    if targets.contains(&Tag::Angular) {
        return "src/**/*.ts".to_string();
    }
    if targets.contains(&Tag::Client) {
        return "src/**/*.tsx".to_string();
    }
    "src/**/*.graphql".to_string()
}

fn config_filename_ok(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("must not be empty".to_string());
    }
    let lower = s.to_lowercase();
    let ok = ["json", "yml", "yaml", "js", "ts"]
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")));
    if !ok {
        return Err("must end with .json, .yml, .yaml, .js, or .ts".to_string());
    }
    Ok(())
}

fn prompt_or_exit<T>(result: Result<T, InquireError>) -> anyhow::Result<T> {
    match result {
        Ok(v) => Ok(v),
        Err(InquireError::OperationCanceled) | Err(InquireError::OperationInterrupted) => {
            std::process::exit(0);
        }
        Err(e) => Err(e.into()),
    }
}
