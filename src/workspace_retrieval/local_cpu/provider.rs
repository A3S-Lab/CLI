use std::sync::Arc;

use a3s_code_core::embedding::EmbeddingProvider;

use super::LocalEmbeddingManifest;

#[cfg(feature = "local-cpu-embedding")]
mod enabled {
    use std::collections::HashMap;
    use std::fmt;
    use std::panic::AssertUnwindSafe;
    use std::sync::{Arc, Mutex, OnceLock};

    use a3s_code_core::embedding::{
        EmbeddingBatchRequest, EmbeddingBatchResponse, EmbeddingProvider,
        EmbeddingProviderDescriptor, EmbeddingProviderError, EmbeddingVector,
    };
    use async_trait::async_trait;
    use fastembed::{
        InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles,
        UserDefinedEmbeddingModel,
    };
    use tokio::sync::Semaphore;
    use tokio_util::sync::CancellationToken;

    #[cfg(test)]
    use super::super::embedding_executor_config;
    use super::super::ensure_runtime_supported;
    use super::super::manifest::{PoolingKind, QuantizationKind};
    use super::LocalEmbeddingManifest;

    const MAX_PROCESS_MODELS: usize = 1;
    const MAX_CONCURRENT_INFERENCES: usize = 1;

    static MODEL_CACHE: OnceLock<Mutex<HashMap<String, Arc<ModelSlot>>>> = OnceLock::new();
    static INFERENCE_PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();

    #[derive(Clone, Copy, Debug)]
    enum LocalEmbeddingFailure {
        ArtifactAdmission,
        ModelInitialization,
        ModelLock,
        InferencePermit,
        Inference,
        InferencePanic,
        InferenceJoin,
    }

    impl LocalEmbeddingFailure {
        fn category(self) -> &'static str {
            match self {
                Self::ArtifactAdmission => "artifact_admission",
                Self::ModelInitialization => "model_initialization",
                Self::ModelLock => "model_lock",
                Self::InferencePermit => "inference_permit",
                Self::Inference => "inference",
                Self::InferencePanic => "inference_panic",
                Self::InferenceJoin => "inference_join",
            }
        }
    }

    struct ModelSlot {
        manifest: LocalEmbeddingManifest,
        intra_threads: usize,
        model: OnceLock<Result<Mutex<TextEmbedding>, LocalEmbeddingFailure>>,
    }

    impl ModelSlot {
        fn new(manifest: LocalEmbeddingManifest, intra_threads: usize) -> Self {
            Self {
                manifest,
                intra_threads,
                model: OnceLock::new(),
            }
        }

        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, LocalEmbeddingFailure> {
            let model = self
                .model
                .get_or_init(|| load_model(&self.manifest, self.intra_threads));
            let model = model.as_ref().map_err(|failure| *failure)?;
            let mut model = model.lock().map_err(|_| LocalEmbeddingFailure::ModelLock)?;
            model
                .embed(texts, None)
                .map_err(|_| LocalEmbeddingFailure::Inference)
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum BlockingInferenceError {
        Cancelled,
        Failed(LocalEmbeddingFailure),
    }

    async fn run_blocking_inference<T, F>(
        permits: Arc<Semaphore>,
        cancellation: CancellationToken,
        operation: F,
    ) -> Result<T, BlockingInferenceError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, LocalEmbeddingFailure> + Send + 'static,
    {
        if cancellation.is_cancelled() {
            return Err(BlockingInferenceError::Cancelled);
        }
        let permit = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(BlockingInferenceError::Cancelled),
            permit = permits.acquire_owned() => permit
                .map_err(|_| BlockingInferenceError::Failed(LocalEmbeddingFailure::InferencePermit))?,
        };
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            std::panic::catch_unwind(AssertUnwindSafe(operation))
                .map_err(|_| LocalEmbeddingFailure::InferencePanic)?
        });
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(BlockingInferenceError::Cancelled),
            result = task => match result {
                Ok(Ok(output)) => Ok(output),
                Ok(Err(failure)) => Err(BlockingInferenceError::Failed(failure)),
                Err(_) => Err(BlockingInferenceError::Failed(LocalEmbeddingFailure::InferenceJoin)),
            },
        }
    }

    #[derive(Clone)]
    pub(super) struct LocalCpuEmbeddingProvider {
        descriptor: EmbeddingProviderDescriptor,
        slot: Arc<ModelSlot>,
    }

    impl fmt::Debug for LocalCpuEmbeddingProvider {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("LocalCpuEmbeddingProvider")
                .field("descriptor", &self.descriptor)
                .field("artifact", &"<content-bound>")
                .finish()
        }
    }

    #[async_trait]
    impl EmbeddingProvider for LocalCpuEmbeddingProvider {
        fn descriptor(&self) -> EmbeddingProviderDescriptor {
            self.descriptor.clone()
        }

        async fn embed(
            &self,
            request: EmbeddingBatchRequest,
            cancellation: CancellationToken,
        ) -> Result<EmbeddingBatchResponse, EmbeddingProviderError> {
            if cancellation.is_cancelled() {
                return Err(EmbeddingProviderError::Cancelled);
            }
            let ids = request
                .inputs()
                .iter()
                .map(|input| Arc::<str>::from(input.id()))
                .collect::<Vec<_>>();
            let texts = request
                .inputs()
                .iter()
                .map(|input| input.text().to_owned())
                .collect::<Vec<_>>();
            let permits = INFERENCE_PERMITS
                .get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_INFERENCES)))
                .clone();
            let slot = Arc::clone(&self.slot);
            let embeddings =
                match run_blocking_inference(permits, cancellation, move || slot.embed(&texts))
                    .await
                {
                    Ok(embeddings) => embeddings,
                    Err(BlockingInferenceError::Cancelled) => {
                        return Err(EmbeddingProviderError::Cancelled);
                    }
                    Err(BlockingInferenceError::Failed(failure)) => {
                        tracing::warn!(
                            provider = "local-cpu",
                            model = %self.descriptor.model,
                            failure = failure.category(),
                            "Local CPU embedding inference failed"
                        );
                        return Err(EmbeddingProviderError::Other);
                    }
                };
            if embeddings.len() != ids.len() {
                return Err(EmbeddingProviderError::Other);
            }
            let vectors = ids
                .into_iter()
                .zip(embeddings)
                .map(|(id, values)| EmbeddingVector::new(id, values))
                .collect();
            Ok(EmbeddingBatchResponse::new(self.descriptor(), vectors))
        }
    }

    pub(super) fn build_provider(
        manifest: LocalEmbeddingManifest,
        intra_threads: usize,
    ) -> anyhow::Result<Arc<dyn EmbeddingProvider>> {
        ensure_runtime_supported()?;
        let descriptor = manifest.descriptor();
        let key = manifest.cache_key(intra_threads);
        let cache = MODEL_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut cache = cache
            .lock()
            .map_err(|_| anyhow::anyhow!("local CPU embedding model cache is unavailable"))?;
        let slot = if let Some(slot) = cache.get(&key) {
            Arc::clone(slot)
        } else {
            if cache.len() >= MAX_PROCESS_MODELS {
                anyhow::bail!("local CPU embedding model cache is limited to one compatible model");
            }
            let slot = Arc::new(ModelSlot::new(manifest, intra_threads));
            cache.insert(key, Arc::clone(&slot));
            slot
        };
        Ok(Arc::new(LocalCpuEmbeddingProvider { descriptor, slot }))
    }

    fn load_model(
        manifest: &LocalEmbeddingManifest,
        intra_threads: usize,
    ) -> Result<Mutex<TextEmbedding>, LocalEmbeddingFailure> {
        let artifacts = manifest
            .admit()
            .map_err(|_| LocalEmbeddingFailure::ArtifactAdmission)?;
        let tokenizer = TokenizerFiles {
            tokenizer_file: artifacts.tokenizer,
            config_file: artifacts.config,
            special_tokens_map_file: artifacts.special_tokens_map,
            tokenizer_config_file: artifacts.tokenizer_config,
        };
        let pooling = match manifest.pooling {
            PoolingKind::Cls => Pooling::Cls,
            PoolingKind::Mean => Pooling::Mean,
        };
        let quantization = match manifest.quantization {
            QuantizationKind::None => QuantizationMode::None,
            QuantizationKind::Static => QuantizationMode::Static,
            QuantizationKind::Dynamic => QuantizationMode::Dynamic,
        };
        let model = UserDefinedEmbeddingModel::new(artifacts.model, tokenizer)
            .with_pooling(pooling)
            .with_quantization(quantization);
        let options = InitOptionsUserDefined::new()
            .with_max_length(manifest.max_length)
            .with_intra_threads(intra_threads);
        TextEmbedding::try_new_from_user_defined(model, options)
            .map(Mutex::new)
            .map_err(|_| LocalEmbeddingFailure::ModelInitialization)
    }

    #[cfg(test)]
    mod tests {
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        use std::time::Duration;
        use std::time::Instant;

        use a3s_code_core::embedding::{EmbeddingError, EmbeddingExecutor, EmbeddingInput};

        use super::*;

        const MAX_REFERENCE_PEAK_RSS_DELTA_BYTES: u64 = 1024 * 1024 * 1024;

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn cancellation_storm_preserves_the_native_inference_bound_and_recovers() {
            let permits = Arc::new(Semaphore::new(1));
            let cancellation = CancellationToken::new();
            let (started_sender, started_receiver) = mpsc::sync_channel(1);
            let (release_sender, release_receiver) = mpsc::sync_channel(1);
            let first = tokio::spawn(run_blocking_inference(
                Arc::clone(&permits),
                cancellation.clone(),
                move || {
                    started_sender.send(()).unwrap();
                    release_receiver.recv().unwrap();
                    Ok(())
                },
            ));
            tokio::task::spawn_blocking(move || {
                started_receiver
                    .recv_timeout(Duration::from_secs(5))
                    .expect("blocking inference did not start")
            })
            .await
            .unwrap();

            cancellation.cancel();
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(1), first)
                    .await
                    .expect("cancelled caller did not return")
                    .unwrap(),
                Err(BlockingInferenceError::Cancelled)
            ));
            assert_eq!(
                permits.available_permits(),
                0,
                "detached native work must retain its concurrency permit"
            );

            let entered = Arc::new(AtomicUsize::new(0));
            let mut cancelled_waiters = Vec::new();
            for _ in 0..32 {
                let waiter_cancellation = CancellationToken::new();
                let waiter_token = waiter_cancellation.clone();
                let waiter_permits = Arc::clone(&permits);
                let waiter_entered = Arc::clone(&entered);
                cancelled_waiters.push(tokio::spawn(async move {
                    run_blocking_inference(waiter_permits, waiter_token, move || {
                        waiter_entered.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                    .await
                }));
                waiter_cancellation.cancel();
            }
            for waiter in cancelled_waiters {
                assert!(matches!(
                    tokio::time::timeout(Duration::from_secs(1), waiter)
                        .await
                        .expect("cancelled queued caller did not return")
                        .unwrap(),
                    Err(BlockingInferenceError::Cancelled)
                ));
            }
            assert_eq!(entered.load(Ordering::SeqCst), 0);

            let recovery_entered = Arc::clone(&entered);
            let recovery = tokio::spawn(run_blocking_inference(
                Arc::clone(&permits),
                CancellationToken::new(),
                move || {
                    recovery_entered.fetch_add(1, Ordering::SeqCst);
                    Ok(7_u8)
                },
            ));
            release_sender.send(()).unwrap();
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(5), recovery)
                    .await
                    .expect("inference queue did not recover")
                    .unwrap(),
                Ok(7)
            ));
            assert_eq!(entered.load(Ordering::SeqCst), 1);
            assert_eq!(permits.available_permits(), 1);
        }

        fn cosine(left: &[f32], right: &[f32]) -> f32 {
            left.iter()
                .zip(right)
                .map(|(left, right)| left * right)
                .sum()
        }

        fn norm(values: &[f32]) -> f32 {
            values.iter().map(|value| value * value).sum::<f32>().sqrt()
        }

        #[cfg(windows)]
        fn peak_rss_bytes() -> u64 {
            use std::mem::{size_of, zeroed};

            use windows_sys::Win32::System::ProcessStatus::{
                GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
            };
            use windows_sys::Win32::System::Threading::GetCurrentProcess;

            // SAFETY: The initialized structure and its exact size are passed
            // to the read-only current-process query for the duration of the call.
            unsafe {
                let mut counters: PROCESS_MEMORY_COUNTERS = zeroed();
                counters.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
                assert_ne!(
                    GetProcessMemoryInfo(
                        GetCurrentProcess(),
                        &mut counters,
                        size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                    ),
                    0
                );
                counters.PeakWorkingSetSize as u64
            }
        }

        #[cfg(unix)]
        fn peak_rss_bytes() -> u64 {
            // SAFETY: getrusage initializes the provided process-local output
            // structure and does not retain its pointer.
            unsafe {
                let mut usage = std::mem::zeroed::<libc::rusage>();
                assert_eq!(libc::getrusage(libc::RUSAGE_SELF, &mut usage), 0);
                #[cfg(target_os = "macos")]
                return usage.ru_maxrss as u64;
                #[cfg(not(target_os = "macos"))]
                return (usage.ru_maxrss as u64).saturating_mul(1024);
            }
        }

        async fn exercise_real_runtime_contract(
            environment_name: &str,
            profile: &str,
            query_text: &str,
            relevant_text: &str,
            distractor_text: &str,
        ) {
            let path = PathBuf::from(
                std::env::var_os(environment_name)
                    .unwrap_or_else(|| panic!("set {environment_name}")),
            );
            let manifest = LocalEmbeddingManifest::load(&path).unwrap();
            let expected_dimension = manifest.dimension();
            let provider = build_provider(manifest, 2).unwrap();
            let executor_config = embedding_executor_config(Duration::from_secs(30));
            let max_batch_inputs = executor_config.max_batch_inputs;
            let executor = EmbeddingExecutor::new(provider, executor_config).unwrap();
            let peak_rss_before = peak_rss_bytes();
            let cold_started = Instant::now();
            let result = executor
                .embed(
                    vec![EmbeddingInput::new("query", query_text)],
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            let cold_ms = cold_started.elapsed().as_millis();
            let query = &result.vectors[0].values;
            assert_eq!(query.len(), expected_dimension);
            assert!((norm(query) - 1.0).abs() < 0.001);
            assert!(cold_ms < 30_000);

            let warm_started = Instant::now();
            let repeated = executor
                .embed(
                    vec![EmbeddingInput::new("query", query_text)],
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            let warm_ms = warm_started.elapsed().as_millis();
            assert!(warm_ms < 5_000);
            assert_eq!(query.len(), repeated.vectors[0].values.len());
            for (first, second) in query.iter().zip(&repeated.vectors[0].values) {
                assert!((first - second).abs() < 0.000_001);
            }

            let candidates = executor
                .embed(
                    vec![
                        EmbeddingInput::new("relevant", relevant_text),
                        EmbeddingInput::new("distractor", distractor_text),
                    ],
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            let relevant_score = cosine(query, &candidates.vectors[0].values);
            let distractor_score = cosine(query, &candidates.vectors[1].values);
            assert!(relevant_score > distractor_score);

            let cancellation = CancellationToken::new();
            let task_cancellation = cancellation.clone();
            let task_executor = executor.clone();
            let cancellation_task = tokio::spawn(async move {
                let inputs = (0..64)
                    .map(|index| {
                        EmbeddingInput::new(
                            format!("cancel-{index}"),
                            "bounded cancellation probe ".repeat(64),
                        )
                    })
                    .collect();
                task_executor.embed(inputs, task_cancellation).await
            });
            tokio::time::sleep(Duration::from_millis(10)).await;
            let cancellation_started = Instant::now();
            cancellation.cancel();
            let cancelled = tokio::time::timeout(Duration::from_secs(1), cancellation_task)
                .await
                .expect("local inference cancellation exceeded one second")
                .expect("local inference cancellation task panicked");
            let cancellation_ms = cancellation_started.elapsed().as_millis();
            assert!(matches!(cancelled, Err(EmbeddingError::Cancelled)));

            let recovery_started = Instant::now();
            let recovery = tokio::time::timeout(
                Duration::from_secs(60),
                executor.embed(
                    vec![EmbeddingInput::new(
                        "post-cancellation",
                        "verify that bounded native inference releases its permit",
                    )],
                    CancellationToken::new(),
                ),
            )
            .await
            .expect("local inference did not recover after cancellation")
            .unwrap();
            let post_cancellation_recovery_ms = recovery_started.elapsed().as_millis();
            assert_eq!(recovery.vectors[0].values.len(), query.len());

            let peak_rss_bytes = peak_rss_bytes();
            let peak_rss_delta_bytes = peak_rss_bytes.saturating_sub(peak_rss_before);
            println!(
                "WSR_LOCAL_CPU_PROVIDER_EVAL={}",
                serde_json::json!({
                    "schemaVersion": 4,
                    "profile": profile,
                    "dimension": query.len(),
                    "coldMs": cold_ms,
                    "warmMs": warm_ms,
                    "cancellationMs": cancellation_ms,
                    "postCancellationRecoveryMs": post_cancellation_recovery_ms,
                    "maxBatchInputs": max_batch_inputs,
                    "peakRssBytes": peak_rss_bytes,
                    "peakRssDeltaBytes": peak_rss_delta_bytes,
                    "queryNorm": norm(query),
                    "relevantScore": relevant_score,
                    "distractorScore": distractor_score,
                    "deterministic": true,
                })
            );
            assert!(
                peak_rss_delta_bytes < MAX_REFERENCE_PEAK_RSS_DELTA_BYTES,
                "local model peak RSS grew by {peak_rss_delta_bytes} bytes from {peak_rss_before} to {peak_rss_bytes}"
            );
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "requires A3S_LOCAL_CPU_MODEL_MANIFEST with admitted ONNX artifacts"]
        async fn real_local_cpu_model_embeds_offline_and_preserves_multilingual_relevance() {
            exercise_real_runtime_contract(
                "A3S_LOCAL_CPU_MODEL_MANIFEST",
                "multilingual-reference",
                "会话结束后，哪个函数负责销毁只存在于内存中的检索投影",
                "pub fn release_ephemeral_projection(generation: &mut Option<u64>) { generation.take(); }",
                "pub fn render_dashboard_color_palette() {}",
            )
            .await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "requires A3S_LOCAL_CPU_SMOKE_MODEL_MANIFEST with admitted ONNX artifacts"]
        async fn real_local_cpu_model_executes_offline_runtime_contract() {
            exercise_real_runtime_contract(
                "A3S_LOCAL_CPU_SMOKE_MODEL_MANIFEST",
                "cross-platform-smoke",
                "Which function releases an in-memory search index when a session closes?",
                "pub fn release_ephemeral_projection(generation: &mut Option<u64>) { generation.take(); }",
                "pub fn render_dashboard_color_palette() {}",
            )
            .await;
        }
    }
}

pub(super) fn build_provider(
    manifest: LocalEmbeddingManifest,
    intra_threads: usize,
) -> anyhow::Result<Arc<dyn EmbeddingProvider>> {
    #[cfg(feature = "local-cpu-embedding")]
    {
        enabled::build_provider(manifest, intra_threads)
    }
    #[cfg(not(feature = "local-cpu-embedding"))]
    {
        let _ = (manifest, intra_threads);
        anyhow::bail!(
            "workspace_retrieval local_cpu requires an a3s binary built with the `local-cpu-embedding` feature"
        )
    }
}
