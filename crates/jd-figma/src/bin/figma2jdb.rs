//! `figma2jdb <figma file URL or key | saved file.json> <out.jdb>`
//!
//! Fetches a Figma file through the REST API (two or so calls, plus one download per image
//! fill) and saves it as a Blobr document. Needs a personal access token in `FIGMA_TOKEN`
//! (Figma: Settings > Security > Personal access tokens, with "File content" read access).
//! The token goes to curl on stdin, so it never shows up in the process list.
//!
//! With a `.json` file instead (the body of `GET /v1/files/:key?geometry=paths`), nothing is
//! fetched and image fills come in grey.

use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

fn main() {
    if let Err(e) = run() {
        eprintln!("figma2jdb: {e}");
        std::process::exit(1);
    }
}

/// A GET through curl, with the token header given on stdin.
fn get(url: &str, token: Option<&str>) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "--fail-with-body", "-L"]);
    if token.is_some() {
        cmd.args(["-H", "@-"]);
    }
    let mut child = cmd.arg(url).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("couldn't run curl: {e}"))?;
    if let Some(t) = token {
        child.stdin.take().expect("piped").write_all(format!("X-Figma-Token: {t}\n").as_bytes()).map_err(|e| e.to_string())?;
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("{url}: {}{}", String::from_utf8_lossy(&out.stderr).trim(), String::from_utf8_lossy(&out.stdout).chars().take(300).collect::<String>()));
    }
    Ok(out.stdout)
}

/// The file key from `https://www.figma.com/design/<key>/...`, or the key itself.
fn file_key(arg: &str) -> &str {
    for marker in ["/design/", "/file/", "/board/"] {
        if let Some(rest) = arg.split(marker).nth(1) {
            return rest.split(['/', '?']).next().unwrap_or(rest);
        }
    }
    arg
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [source, out] = args.as_slice() else {
        return Err("usage: figma2jdb <figma file URL or key | file.json> <out.jdb>".into());
    };
    // Make the output's folder first, so a bad path fails before any API calls.
    if let Some(dir) = std::path::Path::new(out).parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let (json, images) = if source.ends_with(".json") {
        (std::fs::read_to_string(source).map_err(|e| format!("{source}: {e}"))?, HashMap::new())
    } else {
        let token = std::env::var("FIGMA_TOKEN").map_err(|_| "set FIGMA_TOKEN to a Figma personal access token".to_string())?;
        let key = file_key(source);
        eprintln!("Fetching file {key}...");
        let json = String::from_utf8(get(&format!("https://api.figma.com/v1/files/{key}?geometry=paths"), Some(&token))?).map_err(|e| e.to_string())?;
        // Image fills: a map of image ref -> temporary download URL.
        let fills = get(&format!("https://api.figma.com/v1/files/{key}/images"), Some(&token))?;
        let fills: serde_json::Value = serde_json::from_slice(&fills).map_err(|e| e.to_string())?;
        let mut images = HashMap::new();
        if let Some(map) = fills.pointer("/meta/images").and_then(|m| m.as_object()) {
            eprintln!("Downloading {} images...", map.len());
            for (r, url) in map {
                if let Some(url) = url.as_str() {
                    match get(url, None) {
                        Ok(bytes) => {
                            images.insert(r.clone(), bytes);
                        }
                        Err(e) => eprintln!("  skipped an image: {e}"),
                    }
                }
            }
        }
        (json, images)
    };
    let (doc, report) = jd_figma::import(&json, &images)?;
    jd_vector::file::save(&doc, std::path::Path::new(out)).map_err(|e| format!("{out}: {e}"))?;
    eprintln!("Saved {out}: {} pages, {} layers.", report.pages, report.layers);
    let mut notes: Vec<_> = report.notes.into_iter().collect();
    notes.sort();
    for (what, n) in notes {
        eprintln!("  {n} × {what}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_keys_from_urls() {
        assert_eq!(super::file_key("https://www.figma.com/design/y2CmiTTVmgx3mksdEJlvpD/Untitled?node-id=0-1"), "y2CmiTTVmgx3mksdEJlvpD");
        assert_eq!(super::file_key("abc123"), "abc123");
    }
}
