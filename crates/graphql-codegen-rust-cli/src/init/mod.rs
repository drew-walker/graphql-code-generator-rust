mod helpers;
mod plugins;
mod questions;
mod targets;
pub mod types;

use colored::Colorize;
use helpers::{WriteConfigResult, write_config, write_package};
use questions::get_answers;
use targets::guess_targets;
use types::{CodegenConfig, GenerateEntry, InitAnswers, Tag};

use std::collections::HashMap;

fn build_config(answers: &InitAnswers) -> CodegenConfig {
    let needs_documents = answers.targets.contains(&Tag::Client)
        || answers.targets.contains(&Tag::Angular)
        || answers.targets.contains(&Tag::Stencil);

    let plugins: Vec<String> = answers
        .plugins
        .as_ref()
        .map(|ps| ps.iter().map(|p| p.value.clone()).collect())
        .unwrap_or_default();

    let entry = GenerateEntry {
        preset: if answers.targets.contains(&Tag::Client) {
            Some("client".to_string())
        } else {
            None
        },
        plugins,
    };

    let mut generates = HashMap::new();
    generates.insert(answers.output.clone(), entry);

    let documents = if needs_documents {
        answers.documents.clone()
    } else {
        None
    };

    CodegenConfig::new(true, answers.schema.clone(), documents, generates)
}

pub async fn init() -> anyhow::Result<()> {
    println!(
        r#"
        Welcome to {}!
        Answer a few questions and we will setup everything for you.
    "#,
        "GraphQL Code Generator... in Rust 🦀".bold()
    );

    let possible_targets = guess_targets().await?;
    let answers = get_answers(&possible_targets).await?;

    let config = build_config(&answers);

    if answers.introspection {
        // add_introspection(&mut config);
    }

    let WriteConfigResult { relative_path, .. } = write_config(&answers, &config).await?;

    println!("Fetching latest versions of selected plugins...");

    write_package(&answers, &relative_path).await?;

    println!(
        r#"
        Config file generated at {}

          {}

        To install the plugins.

          {}

        To run GraphQL Code Generator.
    "#,
        relative_path.bold(),
        "$ npm install".bold(),
        format!("$ npm run {}", answers.script).bold(),
    );

    Ok(())
}
