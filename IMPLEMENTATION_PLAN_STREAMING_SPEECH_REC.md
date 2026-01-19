# Streaming Speech Recognition Plan (Sherpa-ONNX via sherpa-rs-sys)

## Discussion / decisions to confirm

This section is intentionally conversational. It captures architectural decisions and trade-offs that we should align on before implementing. Please review in-chat and edit in this file as needed.

1) Backend selection and runtime behavior
- Do we want Sherpa-ONNX to be a new `SR_BACKEND` option (e.g. `sherpa_onnx`) alongside Whisper, or do we want to fully replace Whisper for now?
- When Sherpa is enabled, do we still keep the Whisper backend available for offline/final-only use, or is the intent to move all SR to streaming?

2) Endpointing ownership (VAD vs Sherpa endpoint)
- The current pipeline uses `voice_input` VAD to delimit utterances (`AudioEnded`). Sherpa has its own endpoint detector (`enable_endpoint` + rule* params). Using both can cause double-finalization or premature resets.
- Recommendation for first implementation: keep Sherpa endpointing disabled (`enable_endpoint = 0`) and rely on existing VAD boundaries. This avoids conflicts and minimizes behavioral change.
- Alternative: enable Sherpa endpointing and let the recognizer finalize (and reset) mid-stream. If we choose this, we must define how to reconcile `VoiceInputEvent::AudioEnded` with Sherpa endpointing.

3) Partial results delivery
- `SpeechRecEvent::Text { is_final: bool }` already supports partials, but `orchestrator` ignores `is_final=false`. Do we want partials for UI/RPC clients now?
- If yes, we should route partials into the TCP JSON status stream (or another channel) without triggering `process_text()`.
- If no, we can still run streaming internally and only emit final results when VAD ends; partials can be logged but not emitted.

4) Chunk size + latency
- Sherpa streaming works well with 20-50ms audio chunks. Current `CHUNK_SIZE` default is 512 frames; at 16kHz that is 32ms, which is suitable.
- Do we want to change defaults or allow Sherpa-specific chunk sizing via env (e.g., `SR_SHERPA_CHUNK_SIZE_FRAMES`)?

5) Model management
- Sherpa models are multi-file (encoder/decoder/joiner/tokens). We can:
  - Require explicit paths via env vars (simplest).
  - Add a new downloader in `model_download` for Sherpa models (more work).
- For Kroko-ASR (Banafo), the Python script pulls from a private HF repo; we need an HF token if we implement auto-download. Do we want to handle that now or keep it manual?

6) Threading model
- Current `speech_rec` uses a std::thread for Whisper. Sherpa C-API calls should be isolated to one worker thread because the FFI is not guaranteed to be thread-safe.
- Plan: keep a dedicated worker thread owning the recognizer + stream and send it audio/control commands via channels.

If we align on the above, the implementation will be straightforward and stable.

---

## API + functions to implement (with safe Rust wrappers)

This section enumerates the actual C functions we must call and the safe Rust abstractions to build over `sherpa-rs-sys`.

### C API surface (from `sherpa-onnx/c-api/c-api.h`)
Key functions:
- `SherpaOnnxCreateOnlineRecognizer(config)` / `SherpaOnnxDestroyOnlineRecognizer`
- `SherpaOnnxCreateOnlineStream(recognizer)` / `SherpaOnnxDestroyOnlineStream`
- `SherpaOnnxOnlineStreamAcceptWaveform(stream, sample_rate, samples, n)`
- `SherpaOnnxIsOnlineStreamReady(recognizer, stream)`
- `SherpaOnnxDecodeOnlineStream(recognizer, stream)`
- `SherpaOnnxGetOnlineStreamResult(recognizer, stream)` / `SherpaOnnxDestroyOnlineRecognizerResult`
- `SherpaOnnxOnlineStreamInputFinished(stream)`
- `SherpaOnnxOnlineStreamReset(recognizer, stream)`
- (optional) `SherpaOnnxOnlineStreamIsEndpoint(recognizer, stream)`

Config structs:
- `SherpaOnnxOnlineRecognizerConfig`
- `SherpaOnnxOnlineModelConfig` (transducer/paraformer/zipformer2_ctc)
- `SherpaOnnxFeatureConfig`
- `SherpaOnnxOnlineCtcFstDecoderConfig` (if using CTC + fst)

### Safe Rust wrapper module (new)
Create a local module, e.g. `src/sherpa/mod.rs` or `src/sherpa/online.rs`, that wraps the raw FFI:

Types (ownership + Drop):
- `OnlineRecognizer` (owns `*const SherpaOnnxOnlineRecognizer`)
  - `Drop` -> `SherpaOnnxDestroyOnlineRecognizer`
- `OnlineStream` (owns `*const SherpaOnnxOnlineStream` + a backref to recognizer handle only if needed for calls)
  - `Drop` -> `SherpaOnnxDestroyOnlineStream`
- `OnlineResult` (owns `*const SherpaOnnxOnlineRecognizerResult`)
  - `Drop` -> `SherpaOnnxDestroyOnlineRecognizerResult`
  - `text(&self) -> &str` via `CStr`

Safety + ergonomics:
- Use `std::ffi::CString` to store config strings. The config struct must remain valid for the duration of `CreateOnlineRecognizer`. This means `OnlineRecognizer::new` should build `CString`s and keep them in a config struct owned by Rust for the lifetime of the recognizer, or build a temporary, call create, and then drop if the C API copies (it does not promise it copies). So: keep owned `CString` fields in a Rust config wrapper to ensure lifetimes.
- Prefer a `SherpaConfig` builder struct in Rust with owned `CString` fields and a method `as_ffi()` returning a `SherpaOnnxOnlineRecognizerConfig` with raw pointers.
- Do not mark the wrapper structs `Send` or `Sync`. Keep them in the worker thread only.

Example wrapper methods (signatures):
- `OnlineRecognizer::new(config: SherpaOnlineConfig) -> Result<Self, String>`
- `OnlineRecognizer::create_stream(&self) -> OnlineStream`
- `OnlineStream::accept_waveform(&self, sample_rate: i32, samples: &[f32])`
- `OnlineStream::decode_ready(&self, recognizer: &OnlineRecognizer)` (internally loops `IsReady` + `DecodeOnlineStream`)
- `OnlineStream::result(&self, recognizer: &OnlineRecognizer) -> OnlineResult`
- `OnlineStream::input_finished(&self)`
- `OnlineStream::reset(&self, recognizer: &OnlineRecognizer)`
- `OnlineStream::is_endpoint(&self, recognizer: &OnlineRecognizer) -> bool` (optional)

### Integration points in AlicePi

1) New backend in `src/tasks/speech_rec.rs`
- Add a new backend variant, e.g. `Backend::Sherpa(SherpaBackend)`.
- `SherpaBackend` owns the worker thread and a `mpsc` channel for commands, similar to the Whisper worker but streaming aware.

2) New config env vars (example names)
- `SR_BACKEND=sherpa_onnx`
- `SR_SHERPA_TOKENS` (path)
- `SR_SHERPA_ENCODER` (path)
- `SR_SHERPA_DECODER` (path)
- `SR_SHERPA_JOINER` (path)
- `SR_SHERPA_PROVIDER` (cpu/cuda/coreml)
- `SR_SHERPA_THREADS` (num threads)
- `SR_SHERPA_MODEL_TYPE` (e.g. `transducer`, `paraformer`, `zipformer2_ctc`)
- `SR_SHERPA_DECODING_METHOD` (greedy_search / modified_beam_search)
- `SR_SHERPA_MAX_ACTIVE_PATHS` (for beam search)
- `SR_SHERPA_ENABLE_ENDPOINT=0|1` and rule1/2/3 values (if we decide to use it)

3) Speech_rec flow changes (high level)
- Instead of buffering all audio until `AudioEnded`, forward each chunk to the Sherpa worker immediately.
- The worker thread:
  - On `AudioChunk`: `accept_waveform`, then decode while ready, then read the current result. If text changed, emit `SpeechRecEvent::Text { is_final: false }`.
  - On `AudioEnded`: call `input_finished`, decode until not ready, read final result, emit `is_final: true`, then reset or create a new stream.
  - On `Reset`: `OnlineStreamReset` and clear `last_partial_text` state.
- De-dup partials: only emit when the new text differs from the last sent text to avoid spamming.

4) Orchestrator handling for partials
- Currently it ignores `is_final=false`. Decide whether to:
  - Keep ignoring (safe), or
  - Add a separate path to forward partials to the RPC status stream without triggering `process_text()`.

### Additional implementation notes

- Sample rate: Sherpa expects 16 kHz input. In this repo we already resample in `voice_input` to `STREAM_SAMPLE_RATE` (env). For Sherpa, set `STREAM_SAMPLE_RATE=16000` and `STREAM_CHANNELS=1` to minimize internal resampling.
- Result handling: `SherpaOnnxGetOnlineStreamResult` returns a struct whose `text` pointer is valid until you call `SherpaOnnxDestroyOnlineRecognizerResult`. The wrapper should copy the text into a `String` before dropping the result.
- Model selection: Kroko-ASR uses a transducer model (`encoder`/`decoder`/`joiner` + tokens). Ensure `model_type`/config matches a transducer.
- Error handling: FFI functions generally return pointers or int32. If `CreateOnlineRecognizer` returns null, surface a clear error (paths missing, invalid provider, etc.).

---

## Suggested step-by-step plan for the other implementation session

1) Add a local `sherpa` module with safe wrappers over the sys crate
- Implement owned config builder and ensure C strings live for the lifetime of the recognizer.
- Implement `OnlineRecognizer`, `OnlineStream`, and `OnlineResult` with `Drop`.

2) Add `SherpaBackend` to `speech_rec`
- Parse new env config.
- Spawn a worker thread with a command channel (`AudioChunk`, `AudioEnded`, `Reset`, `Shutdown`).
- In the worker thread, keep one `OnlineRecognizer` and one `OnlineStream` per active session (per speech_rec task instance).

3) Wire streaming behavior
- On chunk: accept, decode loop, fetch result, de-dup partial text, emit `is_final=false`.
- On audio end: input_finished, decode loop, fetch final text, emit `is_final=true`, reset stream for next utterance.

4) Decide partial text routing
- If UI/RPC needs partials, update `orchestrator` to forward them to the client channel (not to `process_text`).

5) Add documentation and env defaults
- Document new backend and env vars in README or config docs.

6) Testing (at least manual)
- Use `MOCK_AUDIO_FILE` with a short WAV to verify partial updates and final output.
- Confirm that Reset clears partials and that no double-final occurs.

---

## Open questions to resolve before implementation

- Do we keep endpoint detection disabled in Sherpa and rely on VAD only?
- Should partials be emitted to clients (RPC) or only logged?
- Are Kroko-ASR model files going to be provided manually, or should we add an HF download path with token support?

