//! Mean pooling for transformer embedding outputs.

/// Mean-pool a rank-3 model output `[seq, dim]` using the attention
/// mask to exclude padding tokens. Padding positions (mask == 0)
/// contribute nothing to the sum and are not counted in the divisor.
///
/// Falls back to dividing by `seq` if the mask is all-zero (should
/// not happen with valid input but avoids division by zero).
pub fn mean_pool_with_mask(
    data: &[f32],
    attention_mask: &[i64],
    seq: usize,
    dim: usize,
) -> Vec<f32> {
    let mut pooled = vec![0.0_f32; dim];
    let mut mask_sum = 0.0_f32;
    for s in 0..seq {
        if s < attention_mask.len() && attention_mask[s] == 0 {
            continue;
        }
        mask_sum += 1.0;
        let offset = s * dim;
        for d in 0..dim {
            pooled[d] += data[offset + d];
        }
    }
    let divisor = if mask_sum > 0.0 { mask_sum } else { seq as f32 };
    for v in &mut pooled {
        *v /= divisor;
    }
    pooled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_pool_all_ones_mask_averages_all_tokens() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let mask = vec![1, 1, 1];
        let pooled = mean_pool_with_mask(&data, &mask, 3, 2);
        assert_eq!(pooled, vec![3.0, 4.0]);
    }

    #[test]
    fn mean_pool_excludes_padding_tokens() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let mask = vec![1, 1, 0];
        let pooled = mean_pool_with_mask(&data, &mask, 3, 2);
        assert_eq!(pooled, vec![2.0, 3.0]);
    }

    #[test]
    fn mean_pool_single_token() {
        let data = vec![1.0, 2.0, 3.0];
        let mask = vec![1];
        let pooled = mean_pool_with_mask(&data, &mask, 1, 3);
        assert_eq!(pooled, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn mean_pool_all_padding_falls_back_to_seq_divisor() {
        let data = vec![1.0, 2.0, 3.0, 4.0];
        let mask = vec![0, 0];
        let pooled = mean_pool_with_mask(&data, &mask, 2, 2);
        assert_eq!(pooled, vec![0.0, 0.0]);
    }

    #[test]
    fn mean_pool_mask_shorter_than_seq_uses_seq_for_overflow() {
        let data = vec![1.0, 2.0, 3.0, 4.0];
        let mask = vec![1];
        let pooled = mean_pool_with_mask(&data, &mask, 2, 2);
        assert_eq!(pooled, vec![2.0, 3.0]);
    }
}
