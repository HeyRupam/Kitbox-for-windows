//! How much did the compression actually cost?
//!
//! File size alone doesn't answer that, and asking the user to squint at a
//! before/after is not an answer either. SSIM compares local structure rather
//! than raw pixel difference, which tracks what people notice: 0.99 is
//! indistinguishable, 0.95 is fine for a form photo, below 0.90 starts to show.

use image::{imageops::FilterType, DynamicImage, GrayImage};

/// Structural similarity between two images, 0.0–1.0.
///
/// Images of different dimensions are compared after scaling `b` to match `a`,
/// so a downscaled result is judged on what it looks like at the original size —
/// which is how it will be seen.
pub fn ssim(a: &DynamicImage, b: &DynamicImage) -> f32 {
    let base = a.to_luma8();
    let (w, h) = (base.width(), base.height());
    if w == 0 || h == 0 {
        return 1.0;
    }
    let other = if b.width() == w && b.height() == h {
        b.to_luma8()
    } else {
        b.resize_exact(w, h, FilterType::Triangle).to_luma8()
    };
    ssim_gray(&base, &other)
}

const WINDOW: u32 = 8;
const STRIDE: u32 = 4;

fn ssim_gray(a: &GrayImage, b: &GrayImage) -> f32 {
    let (w, h) = (a.width(), a.height());
    // Constants from Wang et al. 2004, for 8-bit data.
    let c1 = (0.01f64 * 255.0).powi(2);
    let c2 = (0.03f64 * 255.0).powi(2);

    // An image smaller than one window still deserves an answer.
    if w < WINDOW || h < WINDOW {
        return window_ssim(a, b, 0, 0, w, h, c1, c2) as f32;
    }

    let mut total = 0.0f64;
    let mut count = 0u32;
    let mut y = 0;
    while y + WINDOW <= h {
        let mut x = 0;
        while x + WINDOW <= w {
            total += window_ssim(a, b, x, y, WINDOW, WINDOW, c1, c2);
            count += 1;
            x += STRIDE;
        }
        y += STRIDE;
    }
    if count == 0 {
        1.0
    } else {
        (total / count as f64).clamp(0.0, 1.0) as f32
    }
}

#[allow(clippy::too_many_arguments)]
fn window_ssim(
    a: &GrayImage,
    b: &GrayImage,
    x0: u32,
    y0: u32,
    w: u32,
    h: u32,
    c1: f64,
    c2: f64,
) -> f64 {
    let n = (w * h) as f64;
    let (mut sum_a, mut sum_b) = (0.0f64, 0.0f64);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            sum_a += a.get_pixel(x, y)[0] as f64;
            sum_b += b.get_pixel(x, y)[0] as f64;
        }
    }
    let mean_a = sum_a / n;
    let mean_b = sum_b / n;

    let (mut var_a, mut var_b, mut cov) = (0.0f64, 0.0f64, 0.0f64);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let da = a.get_pixel(x, y)[0] as f64 - mean_a;
            let db = b.get_pixel(x, y)[0] as f64 - mean_b;
            var_a += da * da;
            var_b += db * db;
            cov += da * db;
        }
    }
    // Sample variance, matching the reference implementation.
    let denom = (n - 1.0).max(1.0);
    var_a /= denom;
    var_b /= denom;
    cov /= denom;

    ((2.0 * mean_a * mean_b + c1) * (2.0 * cov + c2))
        / ((mean_a.powi(2) + mean_b.powi(2) + c1) * (var_a + var_b + c2))
}

/// Turn a score into the sentence the result card shows.
pub fn describe(score: f32) -> String {
    let pct = (score * 100.0).clamp(0.0, 100.0);
    let verdict = if score >= 0.98 {
        "near identical"
    } else if score >= 0.95 {
        "very close"
    } else if score >= 0.90 {
        "slightly softer"
    } else if score >= 0.80 {
        "visibly compressed"
    } else {
        "heavily compressed"
    };
    format!("{pct:.1}% visually identical — {verdict}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    #[test]
    fn identical_images_score_one() {
        let img = testkit::photo(128, 96);
        assert!((ssim(&img, &img) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn heavy_compression_scores_lower_than_light() {
        let img = testkit::photo(256, 192);
        let light = testkit::reencode_jpeg(&img, 95);
        let heavy = testkit::reencode_jpeg(&img, 15);
        let s_light = ssim(&img, &light);
        let s_heavy = ssim(&img, &heavy);
        assert!(s_light > s_heavy, "light {s_light} should beat heavy {s_heavy}");
        assert!(s_light > 0.9, "quality 95 should stay close to the source, got {s_light}");
    }

    #[test]
    fn a_structurally_different_picture_scores_poorly() {
        let a = testkit::photo(128, 128);
        let mut inverted = a.to_rgb8();
        for p in inverted.pixels_mut() {
            p.0 = [255 - p.0[0], 255 - p.0[1], 255 - p.0[2]];
        }
        let b = DynamicImage::ImageRgb8(inverted);
        let score = ssim(&a, &b);
        assert!(score < 0.5, "an inverted image should not look similar, got {score}");
    }
}
