// Returns (Vec<(BinCenter, Count)>, BinWidth)
pub fn calculate_histogram(latencies: &[f64]) -> (Vec<(f64, f64)>, f64) {
    if latencies.is_empty() {
        return (vec![], 0.0);
    }

    if latencies.len() == 1 {
        return (vec![(latencies[0], 1.0)], 1.0);
    }

    // Find min/max and calculate mean in single pass
    let (min_val, max_val, sum) = latencies
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY, 0.0), |(min, max, sum), &x| {
            (min.min(x), max.max(x), sum + x)
        });

    let range = max_val - min_val;
    if range == 0.0 {
        // All values are identical
        return (vec![(min_val, latencies.len() as f64)], 1.0);
    }

    let mean = sum / latencies.len() as f64;

    // Calculate optimal bin width using Scott's rule
    let bin_width = calculate_scotts_bin_width(latencies, mean, range);
    let num_bins = (range / bin_width).ceil() as usize;

    // Initialize bins with zero counts
    let mut bins = vec![0.0; num_bins];

    // Count data points in each bin
    for &value in latencies {
        let bin_index = ((value - min_val) / bin_width).floor() as usize;
        // Handle edge case where value equals max_val
        let bin_index = bin_index.min(num_bins - 1);
        bins[bin_index] += 1.0;
    }

    // Convert to (bin_center, count) pairs
    let histogram: Vec<(f64, f64)> = bins
        .into_iter()
        .enumerate()
        .map(|(i, count)| {
            let bin_center = min_val + (i as f64 + 0.5) * bin_width;
            (bin_center, count)
        })
        .collect();

    (histogram, bin_width)
}

// Helper function to calculate optimal bin width using Scott's rule
fn calculate_scotts_bin_width(data: &[f64], mean: f64, range: f64) -> f64 {
    let n = data.len() as f64;

    if n < 2.0 {
        return range;
    }

    // Calculate sample standard deviation
    let variance = data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);

    let std_dev = variance.sqrt();

    if std_dev == 0.0 {
        // Fallback to Sturges' rule when std dev is 0
        let num_bins = ((n.log2() + 1.0).ceil() as usize).max(1);
        return range / num_bins as f64;
    }

    // Scott's rule: h = 3.49 * σ / n^(1/3)
    3.49 * std_dev / n.powf(1.0 / 3.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_histogram() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let (histogram, bin_width) = calculate_histogram(&data);

        assert!(bin_width > 0.0);
        assert!(!histogram.is_empty());

        // Verify total count matches input
        let total_count: f64 = histogram.iter().map(|(_, count)| count).sum();
        assert_eq!(total_count, data.len() as f64);
    }

    #[test]
    fn test_identical_values() {
        let data = vec![5.0; 100];
        let (histogram, bin_width) = calculate_histogram(&data);

        assert_eq!(histogram.len(), 1);
        assert_eq!(histogram[0].0, 5.0); // bin center
        assert_eq!(histogram[0].1, 100.0); // count
        assert!(bin_width > 0.0);
    }

    #[test]
    fn test_empty_data() {
        let data: Vec<f64> = vec![];
        let (histogram, bin_width) = calculate_histogram(&data);

        assert!(histogram.is_empty());
        assert_eq!(bin_width, 0.0);
    }

    #[test]
    fn test_normal_distribution() {
        // Generate some normally distributed data
        let data: Vec<f64> = (0..1000)
            .map(|i| (i as f64 - 500.0) / 100.0) // Simple linear transform
            .collect();

        let (histogram, bin_width) = calculate_histogram(&data);

        assert!(bin_width > 0.0);
        assert!(histogram.len() > 5); // Should have reasonable number of bins

        // Verify total count
        let total_count: f64 = histogram.iter().map(|(_, count)| count).sum();
        assert_eq!(total_count, data.len() as f64);
    }
}
