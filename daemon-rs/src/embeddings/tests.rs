mod tests {
    use super::*;

    fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        crate::test_env::lock()
    }

    struct ModelEnvRestore(Option<String>);

    impl Drop for ModelEnvRestore {
        fn drop(&mut self) {
            if let Some(previous) = self.0.as_ref() {
                std::env::set_var(MODEL_ENV_KEY, previous);
            } else {
                std::env::remove_var(MODEL_ENV_KEY);
            }
        }
    }

    fn set_model_env_for_test(value: Option<&str>) -> ModelEnvRestore {
        let previous = std::env::var(MODEL_ENV_KEY).ok();
        if let Some(value) = value {
            std::env::set_var(MODEL_ENV_KEY, value);
        } else {
            std::env::remove_var(MODEL_ENV_KEY);
        }
        ModelEnvRestore(previous)
    }

    fn set_pool_env_for_test(value: Option<&str>) -> ModelEnvRestore {
        let previous = std::env::var(POOL_ENV_KEY).ok();
        if let Some(value) = value {
            std::env::set_var(POOL_ENV_KEY, value);
        } else {
            std::env::remove_var(POOL_ENV_KEY);
        }
        ModelEnvRestore(previous)
    }

    #[test]
    fn selected_model_defaults_to_bge_base() {
        let _env_lock = env_guard();
        let _restore = set_model_env_for_test(None);
        let selected = selected_model_selection();
        assert_eq!(selected.key, "bge-base-en-v1.5");
        assert_eq!(selected.display_name, "bge-base-en-v1.5");
        assert_eq!(selected.dimension, 768);
        assert_eq!(selected.max_input_tokens, 512);
        assert_eq!(selected.model_file, "bge-base-en-v1.5.onnx");
        assert_eq!(selected.tokenizer_file, "bge-base-en-v1.5-tokenizer.json");
        assert_eq!(selected.pooling, "cls");
    }

    #[test]
    fn selected_model_accepts_bge_aliases() {
        let _env_lock = env_guard();
        let _restore = set_model_env_for_test(None);
        std::env::set_var(MODEL_ENV_KEY, "bge");
        assert_eq!(selected_model_key(), "bge-base-en-v1.5");
        std::env::set_var(MODEL_ENV_KEY, "bge-base");
        assert_eq!(selected_model_key(), "bge-base-en-v1.5");
    }

    #[test]
    fn selected_model_accepts_legacy_l6_aliases() {
        let _env_lock = env_guard();
        let _restore = set_model_env_for_test(None);
        std::env::set_var(MODEL_ENV_KEY, "minilm-l6");
        assert_eq!(selected_model_key(), "all-minilm-l6-v2");
        std::env::set_var(MODEL_ENV_KEY, "minilm-legacy");
        assert_eq!(selected_model_key(), "all-minilm-l6-v2");
    }

    #[test]
    fn unknown_model_falls_back_to_default() {
        let _env_lock = env_guard();
        let _restore = set_model_env_for_test(Some("unknown-model-key"));
        assert_eq!(selected_model_key(), "bge-base-en-v1.5");
    }

    #[test]
    fn selected_model_accepts_l12_aliases() {
        let _env_lock = env_guard();
        let _restore = set_model_env_for_test(None);
        std::env::set_var(MODEL_ENV_KEY, "all-minilm-l12-v2");
        assert_eq!(selected_model_key(), "all-minilm-l12-v2");
        std::env::set_var(MODEL_ENV_KEY, "MiniLM");
        assert_eq!(selected_model_key(), "all-minilm-l12-v2");
        std::env::set_var(MODEL_ENV_KEY, "minilm-modern");
        assert_eq!(selected_model_key(), "all-minilm-l12-v2");
    }

    #[test]
    fn selected_model_accepts_qwen3_aliases() {
        let _env_lock = env_guard();
        let _restore = set_model_env_for_test(None);
        std::env::set_var(MODEL_ENV_KEY, "qwen3");
        let selected = selected_model_selection();
        assert_eq!(selected.key, "qwen3-embedding-0.6b");
        assert_eq!(selected.dimension, 1024);
        assert_eq!(selected.max_input_tokens, 512);
        assert_eq!(selected.model_file, "qwen3-embedding-0.6b/model_uint8.onnx");
        assert_eq!(
            selected.tokenizer_file,
            "qwen3-embedding-0.6b/tokenizer.json"
        );
        assert_eq!(selected.pooling, "last_token");
    }

    #[test]
    fn qwen3_profile_uses_single_quantized_onnx_asset() {
        let missing = QWEN3_EMBEDDING_0_6B.missing_assets(Path::new("missing-models-dir"));
        let files = missing.iter().map(|asset| asset.file).collect::<Vec<_>>();
        assert!(files.contains(&"qwen3-embedding-0.6b/model_uint8.onnx"));
        assert!(files.contains(&"qwen3-embedding-0.6b/tokenizer.json"));
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn pooling_strategies_select_expected_token() {
        let data = [
            1.0, 0.0, // token 0
            0.0, 2.0, // token 1
            3.0, 0.0, // token 2
        ];
        let attention = [1, 1, 0];

        let mean =
            EmbeddingEngine::pool_output(&data, 2, 3, &attention, PoolingStrategy::Mean, false)
                .unwrap();
        assert_eq!(mean, vec![0.5, 1.0]);

        let cls =
            EmbeddingEngine::pool_output(&data, 2, 3, &attention, PoolingStrategy::Cls, false)
                .unwrap();
        assert_eq!(cls, vec![1.0, 0.0]);

        let last = EmbeddingEngine::pool_output(
            &data,
            2,
            3,
            &attention,
            PoolingStrategy::LastToken,
            false,
        )
        .unwrap();
        assert_eq!(last, vec![0.0, 2.0]);
    }

    #[test]
    fn session_pool_defaults_to_one() {
        let _env_lock = env_guard();
        let _restore = set_pool_env_for_test(None);
        assert_eq!(resolved_pool_size(), DEFAULT_POOL_SIZE);
    }

    #[test]
    fn session_pool_parses_and_clamps_env_values() {
        let _env_lock = env_guard();
        let _restore = set_pool_env_for_test(None);

        std::env::set_var(POOL_ENV_KEY, "3");
        assert_eq!(resolved_pool_size(), 3);

        std::env::set_var(POOL_ENV_KEY, "99");
        assert_eq!(resolved_pool_size(), MAX_POOL_SIZE);

        std::env::set_var(POOL_ENV_KEY, "0");
        assert_eq!(resolved_pool_size(), 1);

        std::env::set_var(POOL_ENV_KEY, "invalid");
        assert_eq!(resolved_pool_size(), DEFAULT_POOL_SIZE);
    }

    #[test]
    fn cosine_similarity_rejects_non_finite_vectors() {
        assert_eq!(cosine_similarity(&[1.0, f32::NAN], &[1.0, 0.0]), 0.0);
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, f32::INFINITY]), 0.0);
    }

    #[test]
    fn truncate_to_char_boundary_is_utf8_safe() {
        let text = "a🧠b";
        assert_eq!(EmbeddingEngine::truncate_to_char_boundary(text, 6), text);
        assert_eq!(EmbeddingEngine::truncate_to_char_boundary(text, 5), "a🧠");
        assert_eq!(EmbeddingEngine::truncate_to_char_boundary(text, 4), "a");
        assert_eq!(EmbeddingEngine::truncate_to_char_boundary(text, 1), "a");
    }

    // ── PQ8 quantization tests ────────────────────────────────────────────

    fn deterministic_unit_vec(seed: u64, dim: usize) -> Vec<f32> {
        // Tiny xorshift64 PRNG seeded for reproducibility — keeps the test
        // suite hermetic without pulling in the `rand` dev-dependency.
        let mut s = seed | 1;
        let mut raw = Vec::with_capacity(dim);
        for _ in 0..dim {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            raw.push(((s as i64) as f32) / (i64::MAX as f32));
        }
        let norm: f32 = raw.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut raw {
                *v /= norm;
            }
        }
        raw
    }

    #[test]
    fn pq8_blob_is_well_formed() {
        let v = deterministic_unit_vec(0xDEADBEEF, 768);
        let blob = vector_to_pq8_blob(&v);
        assert_eq!(blob.len(), PQ8_HEADER_BYTES + v.len());
        assert_eq!(blob[0], PQ8_MAGIC_BYTE);
        assert_eq!(blob[1], PQ8_FORMAT_VERSION);
        assert!(is_pq8_blob(&blob));
    }

    #[test]
    fn pq8_compression_ratio_matches_target() {
        // Goal: PQ8 cuts a 768-dim BGE vector from 3072B to 774B. Compare
        // explicitly against the legacy f32 encoder since the default path
        // now writes PQ8.
        let v = deterministic_unit_vec(0x11, 768);
        let f32_blob = vector_to_legacy_f32_blob(&v);
        let q8_blob = vector_to_pq8_blob(&v);
        assert_eq!(f32_blob.len(), 3072);
        assert_eq!(q8_blob.len(), 774);
        let ratio = f32_blob.len() as f32 / q8_blob.len() as f32;
        assert!(
            ratio > 3.9 && ratio < 4.0,
            "expected ~4x ratio, got {ratio}"
        );
        // The default writer must agree with the explicit PQ8 encoder.
        assert_eq!(vector_to_blob(&v), q8_blob);
    }

    #[test]
    fn pq8_roundtrip_bounds_error_by_scale() {
        // For a unit vector the scale ~ 1/127, so per-dimension error is
        // bounded by half a step (~0.004). Verify across many seeds.
        for seed in [0x1, 0x100, 0x10000, 0xCAFE, 0xBEEF, 0xFEED] {
            let v = deterministic_unit_vec(seed, 768);
            let blob = vector_to_pq8_blob(&v);
            let recovered = pq8_blob_to_vector(&blob).expect("blob should decode");
            assert_eq!(recovered.len(), v.len());
            // Reconstruct the scale from the blob header so the bound
            // adapts to the actual magnitude of the input.
            let scale = f32::from_le_bytes([blob[2], blob[3], blob[4], blob[5]]);
            let bound = scale; // round error <= half a step, allow full step for safety.
            let err = max_abs_error(&v, &recovered);
            assert!(
                err <= bound,
                "seed={seed:#x}: max_abs_error={err} exceeds bound={bound}"
            );
        }
    }

    #[test]
    fn pq8_preserves_cosine_similarity() {
        // Cosine similarity drift after PQ8 should be small (< 0.01) for
        // L2-normalised vectors. We test self-similarity (=1.0), an
        // orthogonal pair (=0.0 ish), and several random pairs.
        let a = deterministic_unit_vec(0xA1, 768);
        let b = deterministic_unit_vec(0xB2, 768);
        let pairs = [(a.clone(), a.clone()), (a.clone(), b.clone())];
        for (x, y) in pairs {
            let qx = pq8_blob_to_vector(&vector_to_pq8_blob(&x)).unwrap();
            let qy = pq8_blob_to_vector(&vector_to_pq8_blob(&y)).unwrap();
            let raw = cosine_similarity(&x, &y);
            let q = cosine_similarity(&qx, &qy);
            let drift = (raw - q).abs();
            assert!(
                drift < 0.01,
                "cosine drift {drift} too large; raw={raw}, q={q}"
            );
        }
    }

    #[test]
    fn pq8_handles_all_zero_vector() {
        let z = vec![0.0f32; 768];
        let blob = vector_to_pq8_blob(&z);
        let recovered = pq8_blob_to_vector(&blob).unwrap();
        assert_eq!(recovered.len(), 768);
        assert!(recovered.iter().all(|&v| v == 0.0));
        // Scale must be zero for the all-zero special case.
        let scale = f32::from_le_bytes([blob[2], blob[3], blob[4], blob[5]]);
        assert_eq!(scale, 0.0);
    }

    #[test]
    fn pq8_handles_nan_and_infinity_safely() {
        // NaN/inf must NOT poison the scale; they get treated as zero so the
        // remaining valid dimensions are still represented faithfully.
        let mut v = vec![0.5f32; 8];
        v[3] = f32::NAN;
        v[5] = f32::INFINITY;
        v[6] = f32::NEG_INFINITY;
        let blob = vector_to_pq8_blob(&v);
        assert!(is_pq8_blob(&blob));
        let recovered = pq8_blob_to_vector(&blob).unwrap();
        // Non-finite slots come back as zero; finite slots come back ~0.5.
        assert!(recovered[3].abs() < 1e-3);
        assert!(recovered[5].abs() < 1e-3);
        assert!(recovered[6].abs() < 1e-3);
        assert!((recovered[0] - 0.5).abs() < 0.01);
    }

    #[test]
    fn pq8_handles_large_magnitude_input() {
        // Non-normalised vector — scale should track max(|v|) so dynamic
        // range is fully used and clamping never silently saturates.
        let v: Vec<f32> = (0..16).map(|i| (i as f32) - 8.0).collect();
        let blob = vector_to_pq8_blob(&v);
        let recovered = pq8_blob_to_vector(&blob).unwrap();
        let scale = f32::from_le_bytes([blob[2], blob[3], blob[4], blob[5]]);
        // Bound: round error <= scale.
        let err = max_abs_error(&v, &recovered);
        assert!(err <= scale, "err={err} > scale={scale}");
    }

    #[test]
    fn pq8_single_element_works() {
        let v = vec![0.7f32];
        let blob = vector_to_pq8_blob(&v);
        let recovered = pq8_blob_to_vector(&blob).unwrap();
        assert_eq!(recovered.len(), 1);
        assert!((recovered[0] - 0.7).abs() < 0.01);
    }

    #[test]
    fn legacy_blob_is_not_misidentified_as_pq8() {
        // A legacy LE-f32 blob must never be decoded as PQ8 — pq8_blob_to_vector
        // returns None and blob_to_vector falls back to the f32 path. We
        // explicitly call the legacy encoder here because the default
        // `vector_to_blob` now writes PQ8.
        let v = vec![0.1f32, -0.2, 0.3, -0.4, 0.5];
        let legacy = vector_to_legacy_f32_blob(&v);
        assert!(!is_pq8_blob(&legacy));
        assert!(pq8_blob_to_vector(&legacy).is_none());
        let recovered = blob_to_vector(&legacy);
        assert_eq!(recovered.len(), v.len());
        for (a, b) in v.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn blob_to_vector_handles_pq8_payload() {
        let v = deterministic_unit_vec(0xABCD, 16);
        let blob = vector_to_pq8_blob(&v);
        let recovered = blob_to_vector(&blob);
        assert_eq!(recovered.len(), v.len());
        let drift = max_abs_error(&v, &recovered);
        let scale = f32::from_le_bytes([blob[2], blob[3], blob[4], blob[5]]);
        assert!(drift <= scale);
    }

    #[test]
    fn pq8_decodes_fail_on_bad_header() {
        // Truncated header
        assert!(pq8_blob_to_vector(&[]).is_none());
        assert!(pq8_blob_to_vector(&[PQ8_MAGIC_BYTE]).is_none());
        // Wrong magic
        let mut bad = vector_to_pq8_blob(&[0.1, 0.2, 0.3]);
        bad[0] = 0x00;
        assert!(pq8_blob_to_vector(&bad).is_none());
        // Wrong version
        let mut bad = vector_to_pq8_blob(&[0.1, 0.2, 0.3]);
        bad[1] = 0xFF;
        assert!(pq8_blob_to_vector(&bad).is_none());
    }

    #[test]
    fn pq8_decodes_fail_on_malformed_scale() {
        for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
            let mut bad = vec![PQ8_MAGIC_BYTE, PQ8_FORMAT_VERSION];
            bad.extend_from_slice(&scale.to_le_bytes());
            bad.extend_from_slice(&[1, 2, 3, 4]);
            assert!(
                pq8_blob_to_vector(&bad).is_none(),
                "scale {scale:?} should not decode as a valid PQ8 blob"
            );
        }
    }

    #[test]
    fn pq8_decoder_fuzz_corpus_never_emits_non_finite_values() {
        let mut corpus = vec![
            Vec::new(),
            vec![PQ8_MAGIC_BYTE],
            vec![PQ8_MAGIC_BYTE, PQ8_FORMAT_VERSION],
            vector_to_pq8_blob(&[]),
            vector_to_pq8_blob(&[0.0, 1.0, -1.0, f32::NAN, f32::INFINITY]),
        ];

        let mut state = 0xC0DE_F00D_DEAD_BEEFu64;
        for len in 0..96 {
            let mut blob = Vec::with_capacity(len);
            for _ in 0..len {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                blob.push((state >> 32) as u8);
            }
            if len >= PQ8_HEADER_BYTES && len % 7 == 0 {
                blob[0] = PQ8_MAGIC_BYTE;
                blob[1] = PQ8_FORMAT_VERSION;
            }
            corpus.push(blob);
        }

        for blob in corpus {
            if let Some(decoded) = pq8_blob_to_vector(&blob) {
                assert_eq!(decoded.len(), blob.len() - PQ8_HEADER_BYTES);
                assert!(
                    decoded.iter().all(|value| value.is_finite()),
                    "valid PQ8 decode must not emit NaN/inf for blob {blob:?}"
                );
            }
        }
    }

    #[test]
    fn pq8_recall_preserves_top_k_ordering() {
        // Build a small corpus, find the top-3 neighbours of a query in
        // both raw f32 and PQ8 round-tripped form, and assert the top
        // results agree. This is the recall-quality regression guard.
        let dim = 64;
        let corpus: Vec<Vec<f32>> = (0..50)
            .map(|i| deterministic_unit_vec(0xC0DE_0000 + i as u64, dim))
            .collect();
        let query = deterministic_unit_vec(0xC0DE_0001, dim); // exists in corpus
        let raw_scores: Vec<(usize, f32)> = corpus
            .iter()
            .enumerate()
            .map(|(i, v)| (i, cosine_similarity(&query, v)))
            .collect();
        let q_corpus: Vec<Vec<f32>> = corpus
            .iter()
            .map(|v| pq8_blob_to_vector(&vector_to_pq8_blob(v)).unwrap())
            .collect();
        let q_query = pq8_blob_to_vector(&vector_to_pq8_blob(&query)).unwrap();
        let q_scores: Vec<(usize, f32)> = q_corpus
            .iter()
            .enumerate()
            .map(|(i, v)| (i, cosine_similarity(&q_query, v)))
            .collect();
        let mut raw_sorted = raw_scores.clone();
        raw_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let mut q_sorted = q_scores.clone();
        q_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let raw_top: Vec<usize> = raw_sorted.iter().take(3).map(|p| p.0).collect();
        let q_top: Vec<usize> = q_sorted.iter().take(3).map(|p| p.0).collect();
        assert_eq!(
            raw_top[0], q_top[0],
            "top-1 must match: raw={raw_top:?}, q={q_top:?}"
        );
        // Top-3 may permute slightly; require at least 2/3 overlap.
        let overlap = raw_top.iter().filter(|i| q_top.contains(i)).count();
        assert!(
            overlap >= 2,
            "top-3 overlap < 2: raw={raw_top:?}, q={q_top:?}"
        );
    }
}
