use std::env;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use tokio::fs;
use tokio::io::AsyncWriteExt;

struct ModelSpec {
    filename: &'static str,
    url: &'static str,
}

const MODELS: &[ModelSpec] = &[
    ModelSpec {
        filename: "ggml-tiny.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
    },
    ModelSpec {
        filename: "ggml-base.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
    },
    ModelSpec {
        filename: "silero_vad.onnx",
        url: "https://raw.githubusercontent.com/Sameam/whisper_rust/main/models/silero_vad.onnx",
    },
];

pub fn default_models_path(filename: &str) -> PathBuf {
    ggml_dir().join(filename)
}

pub fn default_assets_path(filename: &str) -> PathBuf {
    assets_dir().join(filename)
}

pub async fn ensure_whisper_model(spec: &str) -> Result<(), String> {
    if should_skip_downloads() {
        return Ok(());
    }

    let chosen = parse_model_choice(spec)?;
    let chosen_path = PathBuf::from(&chosen);
    if chosen_path.exists() {
        return Ok(());
    }

    if let Some(filename) = chosen_path.file_name().and_then(|name| name.to_str()) {
        if filename.ends_with(".bin") {
            if let Some(url) = find_url(filename) {
                download_model(url, &chosen_path).await?;
            }
            return Ok(());
        }
    }

    let filename = format!("ggml-{}.bin", chosen);
    if let Some(url) = find_url(&filename) {
        let dest = default_models_path(&filename);
        download_model(url, &dest).await?;
    }

    Ok(())
}

pub async fn ensure_silero_vad(model_path: &Path) -> Result<(), String> {
    if should_skip_downloads() {
        return Ok(());
    }

    if model_path.exists() {
        return Ok(());
    }

    let filename = match model_path.file_name().and_then(|name| name.to_str()) {
        Some(value) => value,
        None => return Ok(()),
    };

    if filename != "silero_vad.onnx" {
        return Ok(());
    }

    if let Some(url) = find_url(filename) {
        download_model(url, model_path).await?;
    }

    Ok(())
}

fn ggml_dir() -> PathBuf {
    env::var("ALICEPI_GGML_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("models"))
}

fn assets_dir() -> PathBuf {
    env::var("ALICEPI_ASSETS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("assets"))
}

fn should_skip_downloads() -> bool {
    env::var("ALICEPI_SKIP_GGML_DOWNLOAD").is_ok()
}

fn parse_model_choice(spec: &str) -> Result<String, String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err("SR_WHISPER_MODEL is empty".to_string());
    }

    let chosen = trimmed
        .split(',')
        .find(|value| !value.trim().is_empty())
        .unwrap_or(trimmed)
        .trim();

    if chosen.is_empty() {
        return Err("SR_WHISPER_MODEL does not contain a usable model path".to_string());
    }

    Ok(chosen.to_string())
}

fn find_url(filename: &str) -> Option<&'static str> {
    MODELS
        .iter()
        .find(|spec| spec.filename == filename)
        .map(|spec| spec.url)
}

async fn download_model(url: &str, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        return Ok(());
    }

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|err| err.to_string())?;
    }

    let response = reqwest::get(url).await.map_err(|err| err.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("download failed for {}: HTTP {}", url, status));
    }

    let temp_path = dest.with_extension("part");
    let mut file = fs::File::create(&temp_path)
        .await
        .map_err(|err| err.to_string())?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|err| err.to_string())?;
        file.write_all(&bytes).await.map_err(|err| err.to_string())?;
    }

    fs::rename(&temp_path, dest)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}
