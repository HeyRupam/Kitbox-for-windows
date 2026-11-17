//! Turning a photograph of a document into something that looks scanned.
//!
//! Three steps, each of which can be done badly in a way that is worse than
//! not doing it:
//!
//! 1. **Find the page.** Photographs of documents are taken at an angle on a
//!    desk, so the page is a quadrilateral somewhere in the frame.
//! 2. **Flatten it.** A homography maps that quadrilateral onto a rectangle.
//! 3. **Clean it up.** Divide out the uneven lighting so the paper reads as
//!    white instead of grey-on-one-side.
//!
//! Detection is the step that fails, and it fails on exactly the photographs
//! people take: a white page on a white table, or a page that fills the frame.
//! So a failure is never silent — the whole frame is used instead, and the
//! report says so for every image. A wrongly cropped scan that looks confident
//! is much worse than one that says it couldn't find the edges.

use image::{DynamicImage, GrayImage, Rgb, RgbImage};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanMode {
    /// Flatten only, colours untouched. For photographs and anything coloured.
    Colour,
    /// Flatten and convert to grey. Smaller, still faithful.
    Greyscale,
    /// Flatten, even out the lighting and push toward black on white. What
    /// people mean by "make it look scanned".
    #[default]
    Document,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanOptions {
    pub mode: ScanMode,
    /// Look for the page's edges. Off, the whole frame is used as-is.
    pub detect_edges: bool,
    /// How hard to push the contrast in document mode, 0.0–1.0.
    pub strength: f32,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions { mode: ScanMode::Document, detect_edges: true, strength: 0.6 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    /// Whether the page's edges were found, or the whole frame was used.
    pub edges_found: bool,
    pub width: u32,
    pub height: u32,
}

/// A quadrilateral in image coordinates: top-left, top-right, bottom-right,
/// bottom-left.
pub type Quad = [(f32, f32); 4];

/// Clean up one photograph.
pub fn scan(bytes: &[u8], options: &ScanOptions) -> Result<(DynamicImage, ScanResult)> {
    if !(0.0..=1.0).contains(&options.strength) {
        return Err(Error::invalid("strength has to be between 0 and 1"));
    }
    let source = image::load_from_memory(bytes)?;
    if source.width() < 16 || source.height() < 16 {
        return Err(Error::invalid("that image is too small to be a page"));
    }
    Ok(scan_image(&source, options))
}

/// The same, on an image already decoded.
pub fn scan_image(source: &DynamicImage, options: &ScanOptions) -> (DynamicImage, ScanResult) {
    let quad = options.detect_edges.then(|| detect_page(source)).flatten();
    let edges_found = quad.is_some();

    let flattened = match quad {
        Some(quad) => warp(source, &quad),
        None => source.to_rgb8(),
    };
    let cleaned = enhance(flattened, options);

    let result = ScanResult {
        edges_found,
        width: cleaned.width(),
        height: cleaned.height(),
    };
    (DynamicImage::ImageRgb8(cleaned), result)
}

/* ---------- finding the page ---------- */

/// The page's four corners, or `None` when nothing convincing was found.
///
/// Works on a downscaled copy: edges that survive being shrunk are the ones
/// that matter, and it makes the search cheap enough to be instant.
pub fn detect_page(source: &DynamicImage) -> Option<Quad> {
    const WORKING: u32 = 480;
    let scale = WORKING as f32 / source.width().max(source.height()) as f32;
    if scale >= 1.0 {
        return detect_in(&source.to_luma8(), 1.0);
    }
    let small = source.resize(
        ((source.width() as f32 * scale) as u32).max(1),
        ((source.height() as f32 * scale) as u32).max(1),
        image::imageops::FilterType::Triangle,
    );
    detect_in(&small.to_luma8(), scale)
}

fn detect_in(grey: &GrayImage, scale: f32) -> Option<Quad> {
    let (w, h) = (grey.width(), grey.height());
    if w < 16 || h < 16 {
        return None;
    }

    // The surroundings are whatever is at the frame's edge; the page is
    // whatever differs from it.
    let (background, spread) = border_statistics(grey);
    let threshold = (spread * 2.0).max(22.0);

    let mut mask: Vec<bool> = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let value = grey.get_pixel(x, y)[0] as f32;
            mask.push((value - background).abs() > threshold);
        }
    }
    // Specks of dust and bright reflections on the desk would otherwise each
    // define a corner, since a corner is by definition an extreme point. A
    // majority filter removes anything that isn't part of a solid region,
    // which lets the corners themselves be taken exactly rather than at a
    // percentile — trimming a percentile pulls every corner inward along its
    // own diagonal, and the page loses its margins.
    let mask = denoise(&mask, w, h);

    // Too little, and there is no page; too much, and the "page" is the whole
    // frame, which is the same as not finding one.
    let set = mask.iter().filter(|m| **m).count();
    let coverage = set as f32 / (w * h) as f32;
    if !(0.12..=0.97).contains(&coverage) {
        return None;
    }

    // The corners of a convex quadrilateral are the extremes of x+y and x-y.
    let mut min_sum = (f32::MAX, (0.0, 0.0));
    let mut max_sum = (f32::MIN, (0.0, 0.0));
    let mut min_diff = (f32::MAX, (0.0, 0.0));
    let mut max_diff = (f32::MIN, (0.0, 0.0));
    for y in 0..h {
        for x in 0..w {
            if !mask[(y * w + x) as usize] {
                continue;
            }
            let (fx, fy) = (x as f32, y as f32);
            let (sum, diff) = (fx + fy, fx - fy);
            if sum < min_sum.0 {
                min_sum = (sum, (fx, fy));
            }
            if sum > max_sum.0 {
                max_sum = (sum, (fx, fy));
            }
            if diff < min_diff.0 {
                min_diff = (diff, (fx, fy));
            }
            if diff > max_diff.0 {
                max_diff = (diff, (fx, fy));
            }
        }
    }

    let top_left = min_sum.1;
    let bottom_right = max_sum.1;
    let top_right = max_diff.1;
    let bottom_left = min_diff.1;

    // Pull the corners a hair inwards. The detected edge sits on the boundary
    // between paper and desk, and that boundary pixel is part desk — left
    // alone it shows up as a dark fringe down two sides of every page, which
    // reads as a defect. The cost is a fraction of a percent of the margin.
    let centre = (
        (top_left.0 + top_right.0 + bottom_right.0 + bottom_left.0) / 4.0,
        (top_left.1 + top_right.1 + bottom_right.1 + bottom_left.1) / 4.0,
    );
    const INSET: f32 = 0.006;
    let pull = |p: (f32, f32)| {
        (p.0 + (centre.0 - p.0) * INSET, p.1 + (centre.1 - p.1) * INSET)
    };

    let quad = [pull(top_left), pull(top_right), pull(bottom_right), pull(bottom_left)];
    if !plausible(&quad, w as f32, h as f32) {
        return None;
    }

    // Back to the original resolution.
    Some(quad.map(|(x, y)| (x / scale, y / scale)))
}

/// Drop any pixel that most of its neighbours disagree with, twice.
///
/// This is what makes taking the true extreme safe: a lone bright speck on the
/// desk cannot survive, so the only things left to be extreme are the page's
/// own corners.
fn denoise(mask: &[bool], w: u32, h: u32) -> Vec<bool> {
    let mut current = mask.to_vec();
    for _ in 0..2 {
        let mut next = vec![false; current.len()];
        for y in 0..h {
            for x in 0..w {
                let mut neighbours = 0;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let nx = x as i32 + dx;
                        let ny = y as i32 + dy;
                        // Outside the frame counts as background, so the page
                        // keeps its shape when it runs off the edge.
                        if nx >= 0
                            && ny >= 0
                            && (nx as u32) < w
                            && (ny as u32) < h
                            && current[(ny as u32 * w + nx as u32) as usize]
                        {
                            neighbours += 1;
                        }
                    }
                }
                next[(y * w + x) as usize] = neighbours >= 5;
            }
        }
        current = next;
    }
    current
}

/// Mean and spread of the pixels around the frame's edge.
fn border_statistics(grey: &GrayImage) -> (f32, f32) {
    let (w, h) = (grey.width(), grey.height());
    let band = (w.min(h) / 24).max(1);
    let mut values = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if x < band || y < band || x >= w - band || y >= h - band {
                values.push(grey.get_pixel(x, y)[0] as f32);
            }
        }
    }
    if values.is_empty() {
        return (255.0, 0.0);
    }
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let variance =
        values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / values.len() as f32;
    (mean, variance.sqrt())
}

/// Reject a "page" that is a sliver, a dot, or wildly lopsided.
fn plausible(quad: &Quad, width: f32, height: f32) -> bool {
    let side = |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    let top = side(quad[0], quad[1]);
    let right = side(quad[1], quad[2]);
    let bottom = side(quad[2], quad[3]);
    let left = side(quad[3], quad[0]);

    let shortest = top.min(right).min(bottom).min(left);
    if shortest < width.min(height) * 0.15 {
        return false;
    }
    // Opposite sides of a page photographed from a sensible angle stay within
    // about a factor of two of each other.
    if top.max(bottom) > top.min(bottom) * 2.5 || left.max(right) > left.min(right) * 2.5 {
        return false;
    }
    true
}

/* ---------- flattening ---------- */

/// Map the quadrilateral onto a rectangle.
fn warp(source: &DynamicImage, quad: &Quad) -> RgbImage {
    let side = |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    // Average the opposite sides: the near edge is longer than the far one, and
    // the truth is between them.
    let width = ((side(quad[0], quad[1]) + side(quad[3], quad[2])) / 2.0).round().max(16.0);
    let height = ((side(quad[0], quad[3]) + side(quad[1], quad[2])) / 2.0).round().max(16.0);
    let (out_w, out_h) = (width as u32, height as u32);

    // Solve for the mapping from the output rectangle back to the photograph,
    // which is the direction sampling needs.
    let destination: Quad = [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)];
    let Some(h) = homography(&destination, quad) else { return source.to_rgb8() };

    let rgb = source.to_rgb8();
    let mut out = RgbImage::new(out_w, out_h);
    for y in 0..out_h {
        for x in 0..out_w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let denominator = h[6] * fx + h[7] * fy + 1.0;
            if denominator.abs() < 1e-6 {
                continue;
            }
            let sx = (h[0] * fx + h[1] * fy + h[2]) / denominator;
            let sy = (h[3] * fx + h[4] * fy + h[5]) / denominator;
            out.put_pixel(x, y, sample(&rgb, sx - 0.5, sy - 0.5));
        }
    }
    out
}

/// The eight coefficients mapping `from` onto `to`, by direct linear transform.
fn homography(from: &Quad, to: &Quad) -> Option<[f32; 8]> {
    // Two equations per corresponding pair.
    let mut m = [[0.0f32; 9]; 8];
    for i in 0..4 {
        let (x, y) = from[i];
        let (u, v) = to[i];
        m[i * 2] = [x, y, 1.0, 0.0, 0.0, 0.0, -x * u, -y * u, u];
        m[i * 2 + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -x * v, -y * v, v];
    }

    // Gaussian elimination with partial pivoting.
    for column in 0..8 {
        let pivot = (column..8)
            .max_by(|a, b| {
                m[*a][column]
                    .abs()
                    .partial_cmp(&m[*b][column].abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })?;
        if m[pivot][column].abs() < 1e-8 {
            return None;
        }
        m.swap(column, pivot);
        let divisor = m[column][column];
        for value in m[column].iter_mut().skip(column) {
            *value /= divisor;
        }
        for row in 0..8 {
            if row == column {
                continue;
            }
            let factor = m[row][column];
            if factor == 0.0 {
                continue;
            }
            for k in column..9 {
                m[row][k] -= factor * m[column][k];
            }
        }
    }

    let mut out = [0.0f32; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = m[i][8];
        if !slot.is_finite() {
            return None;
        }
    }
    Some(out)
}

fn sample(image: &RgbImage, x: f32, y: f32) -> Rgb<u8> {
    let (w, h) = (image.width() as i32, image.height() as i32);
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;

    let at = |px: i32, py: i32| -> [f32; 3] {
        let px = px.clamp(0, w - 1);
        let py = py.clamp(0, h - 1);
        let p = image.get_pixel(px as u32, py as u32).0;
        [p[0] as f32, p[1] as f32, p[2] as f32]
    };

    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    let mut out = [0u8; 3];
    for i in 0..3 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bottom = c[i] + (d[i] - c[i]) * fx;
        out[i] = (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8;
    }
    Rgb(out)
}

/* ---------- cleaning up ---------- */

fn enhance(image: RgbImage, options: &ScanOptions) -> RgbImage {
    match options.mode {
        ScanMode::Colour => image,
        ScanMode::Greyscale => {
            let grey = DynamicImage::ImageRgb8(image).to_luma8();
            DynamicImage::ImageLuma8(grey).to_rgb8()
        }
        ScanMode::Document => document(image, options.strength),
    }
}

/// Even out the lighting, then push toward black on white.
///
/// The background is estimated with a wide box blur and divided out, which is
/// what removes the shadow down one side of a phone photo. A plain contrast
/// curve cannot: it darkens the shadow and the ink together.
fn document(image: RgbImage, strength: f32) -> RgbImage {
    let (w, h) = (image.width(), image.height());
    let grey = DynamicImage::ImageRgb8(image).to_luma8();

    let radius = (w.min(h) / 12).clamp(4, 120);
    let background = box_blur(&grey, radius);

    let mut out = RgbImage::new(w, h);
    // At full strength white starts at 88% of the local background, so paper
    // clips to white while pencil still survives.
    let white_point = 1.0 - 0.12 * strength;
    let black_point = 0.45 - 0.1 * strength;

    for y in 0..h {
        for x in 0..w {
            let value = grey.get_pixel(x, y)[0] as f32;
            let local = background.get_pixel(x, y)[0] as f32;
            // Relative to its own neighbourhood, not to the whole image.
            let ratio = if local > 1.0 { value / local } else { 1.0 };
            let normalised = ((ratio - black_point) / (white_point - black_point)).clamp(0.0, 1.0);
            let level = (normalised * 255.0).round() as u8;
            out.put_pixel(x, y, Rgb([level, level, level]));
        }
    }
    out
}

/// Separable box blur over an integral image: constant time per pixel whatever
/// the radius, which matters because the radius here is large on purpose.
pub(crate) fn box_blur(grey: &GrayImage, radius: u32) -> GrayImage {
    let (w, h) = (grey.width(), grey.height());
    let stride = (w + 1) as usize;
    let mut sums = vec![0u64; stride * (h + 1) as usize];
    for y in 0..h {
        let mut row = 0u64;
        for x in 0..w {
            row += grey.get_pixel(x, y)[0] as u64;
            sums[(y as usize + 1) * stride + x as usize + 1] =
                sums[y as usize * stride + x as usize + 1] + row;
        }
    }

    let mut out = GrayImage::new(w, h);
    for y in 0..h {
        let y0 = y.saturating_sub(radius);
        let y1 = (y + radius + 1).min(h);
        for x in 0..w {
            let x0 = x.saturating_sub(radius);
            let x1 = (x + radius + 1).min(w);
            let total = sums[y1 as usize * stride + x1 as usize]
                + sums[y0 as usize * stride + x0 as usize]
                - sums[y0 as usize * stride + x1 as usize]
                - sums[y1 as usize * stride + x0 as usize];
            let count = ((x1 - x0) * (y1 - y0)) as u64;
            out.put_pixel(x, y, image::Luma([(total / count.max(1)) as u8]));
        }
    }
    out
}

/* ---------- assembling ---------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub pages: u32,
    /// Pages where the edges were found and the page was flattened.
    pub flattened: u32,
    /// Page numbers that were used whole because no edges were found.
    pub whole_frame: Vec<u32>,
    pub note: Option<String>,
}

/// Clean up each photograph and put them into one PDF.
pub fn scan_to_pdf(
    images: &[Vec<u8>],
    options: &ScanOptions,
    pdf: &crate::engine::pdf::ImagesToPdfOptions,
) -> Result<(Vec<u8>, ScanReport)> {
    if images.is_empty() {
        return Err(Error::invalid("no photographs to scan"));
    }

    let mut cleaned = Vec::with_capacity(images.len());
    let mut whole_frame = Vec::new();
    let mut flattened = 0;

    for (index, bytes) in images.iter().enumerate() {
        let (image, result) = scan(bytes, options)?;
        if result.edges_found {
            flattened += 1;
        } else {
            whole_frame.push(index as u32 + 1);
        }
        // Through JPEG because that is what goes into the PDF anyway, and a
        // cleaned-up page compresses well.
        cleaned.push(crate::engine::pdf::images::encode_jpeg_for_pdf(&image, 86, false)?);
    }

    let note = (!whole_frame.is_empty()).then(|| {
        format!(
            "The page edges weren't found on {} of {} photograph{} — those were used whole. \
             That usually means the page fills the frame, or it is the same colour as what it is \
             lying on.",
            whole_frame.len(),
            images.len(),
            if images.len() == 1 { "" } else { "s" }
        )
    });

    let bytes = crate::engine::pdf::images_to_pdf(&cleaned, pdf)?;
    Ok((
        bytes,
        ScanReport { pages: images.len() as u32, flattened, whole_frame, note },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    /// A dark frame with a lighter quadrilateral in it: a page on a desk.
    fn photo_of_a_page(quad: Quad, width: u32, height: u32) -> DynamicImage {
        let mut canvas = RgbImage::from_pixel(width, height, Rgb([42, 40, 38]));
        for y in 0..height {
            for x in 0..width {
                if inside(&quad, x as f32 + 0.5, y as f32 + 0.5) {
                    // Paper, with a lighting gradient across it.
                    let shade = 210.0 + 30.0 * (x as f32 / width as f32);
                    let level = shade.min(255.0) as u8;
                    canvas.put_pixel(x, y, Rgb([level, level, level]));
                }
            }
        }
        DynamicImage::ImageRgb8(canvas)
    }

    fn inside(quad: &Quad, x: f32, y: f32) -> bool {
        // Winding test against the four edges.
        let mut sign = 0;
        for i in 0..4 {
            let (ax, ay) = quad[i];
            let (bx, by) = quad[(i + 1) % 4];
            let cross = (bx - ax) * (y - ay) - (by - ay) * (x - ax);
            let current = if cross > 0.0 { 1 } else { -1 };
            if sign == 0 {
                sign = current;
            } else if sign != current {
                return false;
            }
        }
        true
    }

    fn encode(image: &DynamicImage) -> Vec<u8> {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92)
            .encode_image(&image.to_rgb8())
            .unwrap();
        out
    }

    #[test]
    fn a_page_photographed_at_an_angle_is_found_and_flattened() {
        // A convincing perspective: the top edge shorter than the bottom.
        let quad: Quad = [(120.0, 90.0), (520.0, 130.0), (560.0, 600.0), (80.0, 560.0)];
        let photo = photo_of_a_page(quad, 640, 700);
        let (scanned, result) =
            scan_image(&photo, &ScanOptions { mode: ScanMode::Colour, ..Default::default() });

        assert!(result.edges_found, "the page should be found against the desk");
        // The result is the page alone: nearly all of it is paper, not desk.
        let rgb = scanned.to_rgb8();
        let dark = rgb.pixels().filter(|p| p.0[0] < 120).count();
        assert!(
            (dark as f32 / rgb.pixels().len() as f32) < 0.05,
            "the desk should be gone, {dark} dark pixels remain"
        );
        assert!(result.width > 300 && result.height > 300, "{result:?}");
    }

    #[test]
    fn the_corners_land_on_the_page_rather_than_inside_it() {
        // The regression this guards: taking each corner at a percentile
        // instead of the true extreme pulled all four inward along their own
        // diagonals, which silently shaved the margins off every scan and left
        // the page still skewed.
        let quad: Quad = [(250.0, 190.0), (1180.0, 310.0), (1090.0, 1600.0), (160.0, 1440.0)];
        let photo = photo_of_a_page(quad, 1400, 1800);
        let found = detect_page(&photo).expect("the page should be found");

        for (i, (name, expected)) in
            ["top-left", "top-right", "bottom-right", "bottom-left"].iter().zip(quad).enumerate()
        {
            let (dx, dy) = (found[i].0 - expected.0, found[i].1 - expected.1);
            let off = (dx * dx + dy * dy).sqrt();
            // Detection runs on a downscaled copy, so a few pixels of
            // quantisation is expected; tens of pixels is the bug.
            assert!(off < 20.0, "{name} is {off:.0}px out: {:?} vs {expected:?}", found[i]);
        }
    }

    #[test]
    fn a_frame_with_no_page_in_it_is_used_whole() {
        // Uniform: there is nothing to find.
        let flat = DynamicImage::ImageRgb8(RgbImage::from_pixel(400, 300, Rgb([200, 200, 200])));
        let (scanned, result) =
            scan_image(&flat, &ScanOptions { mode: ScanMode::Colour, ..Default::default() });
        assert!(!result.edges_found, "nothing should have been found");
        assert_eq!((scanned.width(), scanned.height()), (400, 300), "the frame is kept as-is");
    }

    #[test]
    fn a_sliver_is_not_mistaken_for_a_page() {
        // A thin bright strip: too lopsided to be a page.
        let quad: Quad = [(10.0, 140.0), (390.0, 140.0), (390.0, 160.0), (10.0, 160.0)];
        let photo = photo_of_a_page(quad, 400, 300);
        let (_, result) = scan_image(&photo, &ScanOptions::default());
        assert!(!result.edges_found);
    }

    #[test]
    fn detection_can_be_turned_off() {
        let quad: Quad = [(100.0, 80.0), (500.0, 90.0), (510.0, 560.0), (90.0, 550.0)];
        let photo = photo_of_a_page(quad, 640, 640);
        let options = ScanOptions { detect_edges: false, mode: ScanMode::Colour, ..Default::default() };
        let (scanned, result) = scan_image(&photo, &options);
        assert!(!result.edges_found);
        assert_eq!((scanned.width(), scanned.height()), (640, 640));
    }

    #[test]
    fn document_mode_evens_out_the_lighting() {
        // A page lit brightly on one side: the classic phone photo.
        let mut canvas = RgbImage::new(400, 300);
        for (x, _, pixel) in canvas.enumerate_pixels_mut() {
            let shade = (110.0 + 140.0 * (x as f32 / 400.0)) as u8;
            *pixel = Rgb([shade, shade, shade]);
        }
        // Some ink in the dark half.
        for y in 100..140 {
            for x in 40..160 {
                canvas.put_pixel(x, y, Rgb([30, 30, 30]));
            }
        }
        let photo = DynamicImage::ImageRgb8(canvas);

        let options =
            ScanOptions { mode: ScanMode::Document, detect_edges: false, strength: 0.6 };
        let (scanned, _) = scan_image(&photo, &options);
        let rgb = scanned.to_rgb8();

        // Paper on both sides should now read as white.
        let left_paper = rgb.get_pixel(20, 20).0[0];
        let right_paper = rgb.get_pixel(380, 20).0[0];
        assert!(left_paper > 230, "the shadowed side should whiten, got {left_paper}");
        assert!(right_paper > 230, "the bright side too, got {right_paper}");
        // And the ink should still be dark.
        assert!(rgb.get_pixel(100, 120).0[0] < 90, "the writing must survive");
    }

    #[test]
    fn greyscale_mode_removes_the_colour() {
        let mut canvas = RgbImage::from_pixel(80, 60, Rgb([200, 40, 40]));
        canvas.put_pixel(10, 10, Rgb([20, 200, 20]));
        let photo = DynamicImage::ImageRgb8(canvas);
        let options =
            ScanOptions { mode: ScanMode::Greyscale, detect_edges: false, ..Default::default() };
        let (scanned, _) = scan_image(&photo, &options);
        let rgb = scanned.to_rgb8();
        for pixel in rgb.pixels() {
            assert_eq!(pixel.0[0], pixel.0[1], "channels should match in grey");
            assert_eq!(pixel.0[1], pixel.0[2]);
        }
    }

    #[test]
    fn the_homography_round_trips_its_own_corners() {
        let from: Quad = [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)];
        let to: Quad = [(10.0, 20.0), (210.0, 5.0), (190.0, 130.0), (25.0, 115.0)];
        let h = homography(&from, &to).expect("a non-degenerate quad has a solution");
        for i in 0..4 {
            let (x, y) = from[i];
            let d = h[6] * x + h[7] * y + 1.0;
            let u = (h[0] * x + h[1] * y + h[2]) / d;
            let v = (h[3] * x + h[4] * y + h[5]) / d;
            assert!((u - to[i].0).abs() < 0.01, "corner {i} x: {u} vs {}", to[i].0);
            assert!((v - to[i].1).abs() < 0.01, "corner {i} y: {v} vs {}", to[i].1);
        }
    }

    #[test]
    fn a_degenerate_quad_has_no_solution() {
        let collapsed: Quad = [(0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)];
        assert!(homography(&collapsed, &collapsed).is_none());
    }

    #[test]
    fn the_pdf_reports_which_pages_it_could_not_find_edges_on() {
        let quad: Quad = [(120.0, 90.0), (520.0, 130.0), (560.0, 600.0), (80.0, 560.0)];
        let good = encode(&photo_of_a_page(quad, 640, 700));
        let flat = encode(&DynamicImage::ImageRgb8(RgbImage::from_pixel(
            400,
            300,
            Rgb([200, 200, 200]),
        )));

        let (pdf, report) = scan_to_pdf(
            &[good, flat],
            &ScanOptions::default(),
            &crate::engine::pdf::ImagesToPdfOptions::default(),
        )
        .unwrap();

        assert_eq!(report.pages, 2);
        assert_eq!(report.flattened, 1);
        assert_eq!(report.whole_frame, vec![2]);
        let note = report.note.unwrap();
        assert!(note.contains("weren't found"), "{note}");
        assert_eq!(crate::engine::pdf::page_count(&pdf).unwrap(), 2);
    }

    #[test]
    fn bad_input_is_refused() {
        assert!(scan(b"not an image", &ScanOptions::default()).is_err());
        let tiny = encode(&DynamicImage::ImageRgb8(RgbImage::new(8, 8)));
        assert!(scan(&tiny, &ScanOptions::default()).is_err());
        let photo = encode(&testkit::photo(100, 100));
        let loud = ScanOptions { strength: 5.0, ..Default::default() };
        assert!(scan(&photo, &loud).is_err());
    }
}
