#[cfg(feature = "download-models-on-build")]
mod downloader {
    use std::env;
    use std::fs::{self, File};
    use std::io;
    use std::path::{Path, PathBuf};

    const MODELS: &[(&str, &str, &str)] = &[
        (
            "models",
            "ggml-tiny.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        ),
        (
            "models",
            "ggml-base.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        ),
        (
            "assets",
            "silero_vad.onnx",
            "https://raw.githubusercontent.com/Sameam/whisper_rust/main/models/silero_vad.onnx",
        ),
    ];

    pub fn run() {
        println!("cargo:rerun-if-env-changed=ALICEPI_GGML_DIR");
        println!("cargo:rerun-if-env-changed=ALICEPI_ASSETS_DIR");
        println!("cargo:rerun-if-env-changed=ALICEPI_SKIP_GGML_DOWNLOAD");

        if env::var("ALICEPI_SKIP_GGML_DOWNLOAD").is_ok() {
            return;
        }

        let manifest_dir =
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR missing"));
        let models_dir = env::var("ALICEPI_GGML_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| manifest_dir.join("models"));
        let assets_dir = env::var("ALICEPI_ASSETS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| manifest_dir.join("assets"));

        if let Err(err) = ensure_models(&models_dir, &assets_dir) {
            panic!("failed to download ggml models: {err}");
        }
    }

    fn ensure_models(models_dir: &Path, assets_dir: &Path) -> io::Result<()> {
        fs::create_dir_all(models_dir)?;
        fs::create_dir_all(assets_dir)?;
        for (dir, filename, url) in MODELS {
            let base_dir = match *dir {
                "models" => models_dir,
                "assets" => assets_dir,
                _ => models_dir,
            };
            let path = base_dir.join(filename);
            println!("cargo:rerun-if-changed={}", path.display());
            if path.exists() {
                continue;
            }
            download_model(url, &path)
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        }
        Ok(())
    }

    fn download_model(url: &str, dest: &Path) -> Result<(), String> {
        let temp_path = dest.with_extension("part");
        let response = ureq::get(url).call().map_err(|err| err.to_string())?;
        let status = response.status();
        if status != 200 {
            return Err(format!("download failed for {}: HTTP {}", url, status));
        }

        let mut reader = response.into_parts().1.into_reader();
        let mut file = File::create(&temp_path).map_err(|err| err.to_string())?;
        io::copy(&mut reader, &mut file).map_err(|err| err.to_string())?;
        fs::rename(&temp_path, dest).map_err(|err| err.to_string())?;
        Ok(())
    }
}

#[cfg(feature = "download-models-on-build")]
fn main() {
    downloader::run();
}

#[cfg(not(feature = "download-models-on-build"))]
fn main() {}
