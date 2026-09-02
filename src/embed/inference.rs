//! Batched ONNX inference — tokenization, tensor construction, and
//! embedding extraction with mean pooling.

use ort::session::Session;
use ort::value::Tensor;

use super::model::{EmbeddingError, EmbeddingResult};
use super::pooling::mean_pool_with_mask;

/// Run a single batched inference call. Holds the session and tokenizer
/// locks for the duration of inference.
///
/// Parameters:
/// - `session`: loaded ONNX session (mutable to allow `.run()`)
/// - `tokenizer`: loaded HuggingFace tokenizer
/// - `texts`: batch of input texts (already chunked to ≤ CHUNK_SIZE)
/// - `dimension`: expected embedding dimension
/// - `has_token_type_ids`: whether the model accepts token_type_ids input
pub fn run_inference(
    session: &mut Session,
    tokenizer: &tokenizers::Tokenizer,
    texts: &[&str],
    dimension: usize,
    has_token_type_ids: bool,
) -> EmbeddingResult<Vec<Vec<f32>>> {
    // Phase 1: batch tokenize all texts. The tokenizer handles
    // truncation and padding automatically (configured at load time).
    let encodings = tokenizer
        .encode_batch(texts.to_vec(), true)
        .map_err(|e| EmbeddingError::InferenceFailed(format!("tokenization: {}", e)))?;

    // Phase 2: build batch tensors from encoded results.
    let batch_size = texts.len() as i64;
    let padded_len = encodings[0].get_ids().len() as i64;

    let mut batch_input_ids = Vec::with_capacity(texts.len() * padded_len as usize);
    let mut batch_attention_mask = Vec::with_capacity(texts.len() * padded_len as usize);
    let mut batch_token_type_ids = Vec::with_capacity(texts.len() * padded_len as usize);
    let mut all_attention_masks: Vec<Vec<i64>> = Vec::with_capacity(texts.len());

    for encoded in &encodings {
        let input_ids: Vec<i64> = encoded.get_ids().iter().map(|&v| v as i64).collect();
        let attention_mask: Vec<i64> = encoded
            .get_attention_mask()
            .iter()
            .map(|&v| v as i64)
            .collect();
        let token_type_ids: Vec<i64> = encoded.get_type_ids().iter().map(|&v| v as i64).collect();

        all_attention_masks.push(attention_mask.clone());
        batch_input_ids.extend_from_slice(&input_ids);
        batch_attention_mask.extend_from_slice(&attention_mask);
        batch_token_type_ids.extend_from_slice(&token_type_ids);
    }

    let input_ids_tensor = Tensor::from_array((vec![batch_size, padded_len], batch_input_ids))
        .map_err(|e| EmbeddingError::InferenceFailed(format!("input tensor: {}", e)))?;
    let attention_mask_tensor =
        Tensor::from_array((vec![batch_size, padded_len], batch_attention_mask.clone()))
            .map_err(|e| EmbeddingError::InferenceFailed(format!("mask tensor: {}", e)))?;

    // Phase 3: single batched inference call.
    let outputs = if has_token_type_ids {
        let token_type_ids_tensor =
            Tensor::from_array((vec![batch_size, padded_len], batch_token_type_ids))
                .map_err(|e| EmbeddingError::InferenceFailed(format!("type tensor: {}", e)))?;
        session
            .run(ort::inputs![
                input_ids_tensor,
                attention_mask_tensor,
                token_type_ids_tensor
            ])
            .map_err(|e| EmbeddingError::InferenceFailed(format!("inference: {}", e)))?
    } else {
        session
            .run(ort::inputs![input_ids_tensor, attention_mask_tensor])
            .map_err(|e| EmbeddingError::InferenceFailed(format!("inference: {}", e)))?
    };

    // Phase 4: extract and pool embeddings from batched output.
    let (shape, data) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| EmbeddingError::InferenceFailed(format!("extract: {}", e)))?;

    let mut results = Vec::with_capacity(texts.len());

    if shape.len() == 3 {
        // Shape: [batch, seq_len, dim]
        let dim = shape[2] as usize;
        let seq = shape[1] as usize;
        let batch = shape[0] as usize;
        let row_size = seq * dim;

        for i in 0..batch {
            let row = &data[i * row_size..(i + 1) * row_size];
            let mask = &all_attention_masks[i];
            let embedding = mean_pool_with_mask(row, mask, seq, dim);

            if embedding.len() != dimension {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: dimension,
                    actual: embedding.len(),
                });
            }
            results.push(embedding);
        }
    } else if shape.len() == 2 {
        // Shape: [batch, dim] — already pooled
        let dim = shape[1] as usize;
        let batch = shape[0] as usize;

        for i in 0..batch {
            let embedding = data[i * dim..(i + 1) * dim].to_vec();
            if embedding.len() != dimension {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: dimension,
                    actual: embedding.len(),
                });
            }
            results.push(embedding);
        }
    } else {
        return Err(EmbeddingError::InferenceFailed(format!(
            "unexpected output rank: {}",
            shape.len()
        )));
    }

    Ok(results)
}
