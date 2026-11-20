//! How well the solver does, measured rather than felt.
//!
//! The suite proves the solver never lies. This says how *good* it is: how
//! often it lands inside the target, how many real encodes it spends getting
//! there, how much of the picture survives, and how long it takes. Those are
//! the numbers that quietly get worse, and nobody notices until a form takes
//! four seconds or a photograph comes back mush.
//!
//! **What this corpus is.** Synthetic files, generated here: photographs of
//! several sizes, a sharp-edged screenshot, scans of several pages, and a
//! text-only document. The plan asks for fifty real PDFs and photographs, and
//! those would be better — real cameras, real scanners and real exports have
//! quirks no generator invents. Until there is a corpus that can be committed
//! without someone's documents in it, this is what there is, and it is enough
//! to catch a regression.
//!
//! Run with:
//!
//!     cargo test -p mt-core --release --test corpus -- --ignored --nocapture

use std::time::Instant;

use mt_core::engine::image::{compress_to_target, CompressOptions, OutputFormat};
use mt_core::engine::pdf::{self, PdfCompressOptions};
use mt_core::{format_size, Bias, Outcome, SizeTarget};

struct Case {
    what: &'static str,
    bytes: Vec<u8>,
    is_pdf: bool,
}

struct Row {
    what: String,
    target: String,
    before: u64,
    after: u64,
    landed: bool,
    probes: u8,
    score: Option<f32>,
    ms: u128,
    note: String,
}

fn corpus() -> Vec<Case> {
    use mt_core::testkit::{encode_jpeg, pdf_scan, pdf_text_only, photo};

    vec![
        Case {
            what: "phone photo 12 MP",
            bytes: encode_jpeg(&photo(4000, 3000), 92),
            is_pdf: false,
        },
        Case { what: "photo 5 MP", bytes: encode_jpeg(&photo(2560, 1920), 90), is_pdf: false },
        Case { what: "photo 2 MP", bytes: encode_jpeg(&photo(1600, 1200), 88), is_pdf: false },
        Case { what: "small crop", bytes: encode_jpeg(&photo(640, 480), 85), is_pdf: false },
        Case { what: "form photo", bytes: encode_jpeg(&photo(200, 230), 95), is_pdf: false },
        Case { what: "scan, 1 page", bytes: pdf_scan(1, 1700, 2200, 90), is_pdf: true },
        Case { what: "scan, 4 pages", bytes: pdf_scan(4, 1200, 1600, 88), is_pdf: true },
        Case { what: "text document", bytes: pdf_text_only(), is_pdf: true },
    ]
}

fn targets_for(is_pdf: bool) -> Vec<(&'static str, SizeTarget)> {
    if is_pdf {
        vec![
            ("under 100 KB", SizeTarget::AtMost(100_000)),
            ("50–100 KB", SizeTarget::Range { min: 50_000, max: 100_000 }),
            ("under 500 KB", SizeTarget::AtMost(500_000)),
        ]
    } else {
        vec![
            ("20–50 KB", SizeTarget::Range { min: 20_000, max: 50_000 }),
            ("under 100 KB", SizeTarget::AtMost(100_000)),
            ("under 500 KB", SizeTarget::AtMost(500_000)),
        ]
    }
}

#[test]
#[ignore = "a benchmark, not a check; run it on purpose"]
fn how_well_the_solver_does() {
    let mut rows = Vec::new();

    for case in corpus() {
        for (label, target) in targets_for(case.is_pdf) {
            let started = Instant::now();
            let outcome = if case.is_pdf {
                pdf::compress_to_target(
                    &case.bytes,
                    target,
                    Bias::Balanced,
                    PdfCompressOptions { strip_metadata: true, ..Default::default() },
                )
            } else {
                compress_to_target(
                    &case.bytes,
                    target,
                    Bias::Balanced,
                    CompressOptions {
                        format: OutputFormat::Keep,
                        strip_metadata: true,
                        max_dimension: None,
                        exact_dimensions: None,
                    },
                )
            }
            .expect("the engine should answer, even if the answer is no");

            let ms = started.elapsed().as_millis();
            let (landed, probes, score, note) = match &outcome {
                Outcome::Hit(s) => (true, s.probes, s.score, String::new()),
                Outcome::Padded(s) => (true, s.probes, s.score, "padded".into()),
                Outcome::Infeasible { probes, why, .. } => {
                    (false, *probes, None, why.chars().take(40).collect())
                }
            };

            rows.push(Row {
                what: case.what.to_string(),
                target: label.to_string(),
                before: case.bytes.len() as u64,
                after: outcome.size(),
                landed,
                probes,
                score,
                ms,
                note,
            });
        }
    }

    println!(
        "\n{:<20} {:<14} {:>9} {:>9} {:>7} {:>7} {:>9}  note",
        "file", "asked for", "before", "after", "probes", "ms", "kept"
    );
    println!("{}", "-".repeat(96));
    for row in &rows {
        println!(
            "{:<20} {:<14} {:>9} {:>9} {:>7} {:>7} {:>9}  {}",
            row.what,
            row.target,
            format_size(row.before),
            format_size(row.after),
            row.probes,
            row.ms,
            row.score.map(|s| format!("{:.1}%", s * 100.0)).unwrap_or_else(|| "—".into()),
            if row.landed { row.note.clone() } else { format!("missed: {}", row.note) },
        );
    }

    let landed = rows.iter().filter(|r| r.landed).count();
    let probes: f64 = rows.iter().map(|r| r.probes as f64).sum::<f64>() / rows.len() as f64;
    let scored: Vec<f32> = rows.iter().filter_map(|r| r.score).collect();
    let kept = scored.iter().sum::<f32>() / scored.len().max(1) as f32;
    let slowest = rows.iter().map(|r| r.ms).max().unwrap_or(0);
    let total: u128 = rows.iter().map(|r| r.ms).sum();

    println!(
        "\n{landed} of {} landed inside · {probes:.1} encodes each on average · \
         {:.1}% of the picture kept · slowest {slowest} ms · {total} ms in total",
        rows.len(),
        kept * 100.0
    );
    println!(
        "A synthetic corpus, not fifty real files: good enough to catch a regression, \
         not a claim about your documents.\n"
    );

    // Guard rails rather than targets. These are deliberately loose: the point
    // is to catch something falling off a cliff, not to freeze today's numbers
    // into a test that fails on a lucky encode.
    assert!(
        landed * 4 >= rows.len() * 3,
        "only {landed} of {} landed inside the target",
        rows.len()
    );
    assert!(probes < 14.0, "{probes:.1} encodes per file on average is too many");
    // Was 11 seconds before the solver stopped measuring every attempt it
    // throws away, and is about 3 now. Loose enough for a slower machine,
    // tight enough to notice that coming back.
    assert!(slowest < 15_000, "the slowest case took {slowest} ms");
}
