use anyhow::Context;
use serde_json::Value;

/// Fetches the version directly from the registry instead of depending on
/// an ESM-only module as latest-version does.
pub fn get_latest_version(package_name: &str) -> anyhow::Result<String> {
    let url = format!("https://unpkg.com/{package_name}/package.json");

    let body = ureq::get(&url)
        .call()
        .context("request unpkg")?
        .into_body()
        .read_to_string()
        .context("read unpkg body")?;

    let v: Value = serde_json::from_str(&body).context("parse unpkg json")?;

    let version = v
        .get("version")
        .and_then(|x| x.as_str())
        .context("missing version in unpkg response")?;

    Ok(version.to_string())
}
