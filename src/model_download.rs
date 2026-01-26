use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bzip2::read::BzDecoder;
use futures_util::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tar::Archive;
use tokio::fs;
use tokio::io::AsyncWriteExt;

struct ModelSpec {
    filename: &'static str,
    url: &'static str,
}

struct SherpaZipformerPreset {
    name: &'static str,
    archive: &'static str,
    url: &'static str,
    dir: &'static str,
    encoder_fp32: &'static str,
    decoder_fp32: &'static str,
    joiner_fp32: &'static str,
    encoder_int8: &'static str,
    joiner_int8: &'static str,
    tokens: &'static str,
    bpe_vocab: Option<&'static str>,
    modeling_unit: Option<&'static str>,
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
        filename: "ggml-base.en.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
    },
    ModelSpec {
        filename: "silero_vad.onnx",
        url: "https://raw.githubusercontent.com/Sameam/whisper_rust/main/models/silero_vad.onnx",
    },
];

const SHERPA_ZIPFORMER_PRESETS: &[SherpaZipformerPreset] = &[SherpaZipformerPreset {
    name: "zipformer-en-20M-2023-02-17",
    archive: "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17.tar.bz2",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17.tar.bz2",
    dir: "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17",
    encoder_fp32: "encoder-epoch-99-avg-1.onnx",
    decoder_fp32: "decoder-epoch-99-avg-1.onnx",
    joiner_fp32: "joiner-epoch-99-avg-1.onnx",
    encoder_int8: "encoder-epoch-99-avg-1.int8.onnx",
    joiner_int8: "joiner-epoch-99-avg-1.int8.onnx",
    tokens: "tokens.txt",
    bpe_vocab: None,
    modeling_unit: None,
}];

#[allow(dead_code)]
pub struct SherpaZipformerPaths {
    pub dir: PathBuf,
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub joiner: PathBuf,
    pub tokens: PathBuf,
    pub bpe_vocab: Option<PathBuf>,
    pub modeling_unit: Option<&'static str>,
}

struct DownloadPlan {
    url: &'static str,
    dest: PathBuf,
    label: String,
}

struct DownloadProgress {
    bar: ProgressBar,
    label: String,
    bar_style: ProgressStyle,
}

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

    if let Some(plan) = whisper_download_plan(spec)? {
        println!(
            "Downloading model from {} to {}",
            plan.url,
            plan.dest.display()
        );
        download_model(plan.url, &plan.dest, None).await?;
    }

    Ok(())
}

pub async fn ensure_sherpa_zipformer_model(
    name: &str,
    variant: &str,
    model_dir: Option<&Path>,
) -> Result<Option<SherpaZipformerPaths>, String> {
    let paths = match sherpa_zipformer_paths(name, variant, model_dir)? {
        Some(paths) => paths,
        None => return Ok(None),
    };

    if should_skip_downloads() {
        return Ok(Some(paths));
    }

    if sherpa_zipformer_files_exist(&paths) {
        return Ok(Some(paths));
    }

    let preset = sherpa_zipformer_preset(name).ok_or_else(|| {
        format!(
            "unknown sherpa zipformer preset '{}'; set SR_SHERPA_MODEL_DIR or explicit paths",
            name
        )
    })?;

    let output_dir = match model_dir {
        Some(dir) => {
            let dir_name = dir.file_name().and_then(|name| name.to_str()).unwrap_or("");
            if !dir_name.is_empty() && dir_name != preset.dir {
                return Err(format!(
                    "SR_SHERPA_MODEL_DIR must end with '{}' to auto-download preset '{}'",
                    preset.dir, preset.name
                ));
            }
            dir.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        }
        None => ggml_dir(),
    };

    let archive_path = output_dir.join(preset.archive);
    if !archive_path.exists() {
        println!(
            "Downloading sherpa zipformer model from {} to {}",
            preset.url,
            archive_path.display()
        );
        download_model(preset.url, &archive_path, None).await?;
    }

    extract_tar_bz2(&archive_path, &output_dir).await?;
    let _ = fs::remove_file(&archive_path).await;

    Ok(Some(paths))
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
        download_model(url, model_path, None).await?;
    }

    Ok(())
}

pub async fn ensure_models_with_progress(
    whisper_spec: &str,
    silero_path: &Path,
) -> Result<(), String> {
    if should_skip_downloads() {
        return Ok(());
    }

    let whisper_plan = whisper_download_plan(whisper_spec)?;
    let silero_plan = silero_download_plan(silero_path)?;

    if whisper_plan.is_none() && silero_plan.is_none() {
        return Ok(());
    }

    let progress = MultiProgress::new();
    let spinner_style = ProgressStyle::with_template("{msg:20} {spinner} {bytes}")
        .map_err(|err| err.to_string())?;
    let bar_style =
        ProgressStyle::with_template("{msg:20} {bar:40.cyan/blue} {bytes}/{total_bytes} ({eta})")
            .map_err(|err| err.to_string())?
            .progress_chars("##-");

    let whisper_future = {
        let progress = &progress;
        let spinner_style = spinner_style.clone();
        let bar_style = bar_style.clone();
        async move {
            if let Some(plan) = whisper_plan {
                let bar = progress.add(ProgressBar::new_spinner());
                bar.set_message(plan.label.clone());
                bar.set_style(spinner_style);
                bar.enable_steady_tick(Duration::from_millis(120));
                let progress = DownloadProgress {
                    bar,
                    label: plan.label,
                    bar_style,
                };
                download_model(plan.url, &plan.dest, Some(progress)).await?;
            }
            Ok::<(), String>(())
        }
    };

    let silero_future = {
        let progress = &progress;
        let spinner_style = spinner_style.clone();
        let bar_style = bar_style.clone();
        async move {
            if let Some(plan) = silero_plan {
                let bar = progress.add(ProgressBar::new_spinner());
                bar.set_message(plan.label.clone());
                bar.set_style(spinner_style);
                bar.enable_steady_tick(Duration::from_millis(120));
                let progress = DownloadProgress {
                    bar,
                    label: plan.label,
                    bar_style,
                };
                download_model(plan.url, &plan.dest, Some(progress)).await?;
            }
            Ok::<(), String>(())
        }
    };

    tokio::try_join!(whisper_future, silero_future).map(|_| ())
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

pub fn should_skip_downloads() -> bool {
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

    let filename = format!("ggml-{}.bin", chosen);
    Ok(filename)
}

fn whisper_download_plan(spec: &str) -> Result<Option<DownloadPlan>, String> {
    let chosen = parse_model_choice(spec)?;
    let chosen_path = PathBuf::from(default_models_path(&chosen));
    if chosen_path.exists() {
        return Ok(None);
    }

    if let Some(filename) = chosen_path.file_name().and_then(|name| name.to_str()) {
        let label = format!("GGML {}", filename);
        if filename.ends_with(".bin") {
            if let Some(url) = find_url(filename) {
                return Ok(Some(DownloadPlan {
                    url,
                    label,
                    dest: chosen_path,
                }));
            }
            return Ok(None);
        }
    }

    if let Some(url) = find_url(&chosen) {
        return Ok(Some(DownloadPlan {
            url,
            dest: default_models_path(&chosen),
            label: format!("GGML {}", chosen),
        }));
    }

    Ok(None)
}

fn silero_download_plan(model_path: &Path) -> Result<Option<DownloadPlan>, String> {
    if model_path.exists() {
        return Ok(None);
    }

    let filename = match model_path.file_name().and_then(|name| name.to_str()) {
        Some(value) => value,
        None => return Ok(None),
    };

    if filename != "silero_vad.onnx" {
        return Ok(None);
    }

    if let Some(url) = find_url(filename) {
        return Ok(Some(DownloadPlan {
            url,
            dest: model_path.to_path_buf(),
            label: "Silero VAD".to_string(),
        }));
    }

    Ok(None)
}

fn find_url(filename: &str) -> Option<&'static str> {
    println!("Finding URL for filename: {}", filename);
    MODELS
        .iter()
        .find(|spec| spec.filename == filename)
        .map(|spec| spec.url)
        .or_else(|| {
            panic!("Requested model {:?} does not exist and I don't know how to download it! Download it yourself and place it at {}", filename, default_models_path(filename).display())
        })
}

fn sherpa_zipformer_preset(name: &str) -> Option<&'static SherpaZipformerPreset> {
    let trimmed = name.trim();
    SHERPA_ZIPFORMER_PRESETS.iter().find(|preset| {
        preset.name.eq_ignore_ascii_case(trimmed) || preset.dir.eq_ignore_ascii_case(trimmed)
    })
}

pub fn sherpa_zipformer_paths(
    name: &str,
    variant: &str,
    model_dir: Option<&Path>,
) -> Result<Option<SherpaZipformerPaths>, String> {
    let preset = match sherpa_zipformer_preset(name) {
        Some(preset) => preset,
        None => return Ok(None),
    };

    let variant = variant.trim().to_lowercase();
    let (encoder_name, joiner_name) = match variant.as_str() {
        "fp32" | "" => (preset.encoder_fp32, preset.joiner_fp32),
        "int8" => (preset.encoder_int8, preset.joiner_int8),
        other => {
            return Err(format!(
                "unsupported sherpa zipformer variant '{}'; use 'fp32' or 'int8'",
                other
            ))
        }
    };

    let base_dir = match model_dir {
        Some(dir) => dir.to_path_buf(),
        None => default_models_path(preset.dir),
    };

    let bpe_vocab = preset
        .bpe_vocab
        .map(|filename| base_dir.join(filename));

    Ok(Some(SherpaZipformerPaths {
        dir: base_dir.clone(),
        encoder: base_dir.join(encoder_name),
        decoder: base_dir.join(preset.decoder_fp32),
        joiner: base_dir.join(joiner_name),
        tokens: base_dir.join(preset.tokens),
        bpe_vocab,
        modeling_unit: preset.modeling_unit,
    }))
}

fn sherpa_zipformer_files_exist(paths: &SherpaZipformerPaths) -> bool {
    if !paths.encoder.exists()
        || !paths.decoder.exists()
        || !paths.joiner.exists()
        || !paths.tokens.exists()
    {
        return false;
    }
    if let Some(ref vocab) = paths.bpe_vocab {
        if !vocab.exists() {
            return false;
        }
    }
    true
}

async fn download_model(
    url: &str,
    dest: &Path,
    progress: Option<DownloadProgress>,
) -> Result<(), String> {
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

    let mut progress = progress;
    if let Some(ref mut progress) = progress {
        if let Some(total) = response.content_length() {
            progress.bar.set_style(progress.bar_style.clone());
            progress.bar.set_length(total);
            progress.bar.disable_steady_tick();
        }
    }

    let temp_path = dest.with_extension("part");
    let mut file = fs::File::create(&temp_path)
        .await
        .map_err(|err| err.to_string())?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|err| err.to_string())?;
        file.write_all(&bytes).await.map_err(|err| err.to_string())?;
        if let Some(ref progress) = progress {
            progress.bar.inc(bytes.len() as u64);
        }
    }

    fs::rename(&temp_path, dest)
        .await
        .map_err(|err| err.to_string())?;

    if let Some(progress) = progress {
        progress
            .bar
            .finish_with_message(format!("{} done", progress.label));
    }
    Ok(())
}

async fn extract_tar_bz2(archive_path: &Path, output_dir: &Path) -> Result<(), String> {
    let archive_path = archive_path.to_path_buf();
    let output_dir = output_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&archive_path)
            .map_err(|err| format!("open archive {} failed: {}", archive_path.display(), err))?;
        let decompressor = BzDecoder::new(file);
        let mut archive = Archive::new(decompressor);
        archive
            .unpack(&output_dir)
            .map_err(|err| format!("extract archive failed: {}", err))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|err| err.to_string())?
}
