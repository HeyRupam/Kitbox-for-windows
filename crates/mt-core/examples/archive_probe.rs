//! Check or convert a PDF for archiving.
//!
//! cargo run -p mt-core --example archive_probe -- in.pdf [out.pdf] [icc.icm]
fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("a pdf");
    let out = args.next();
    let icc = args.next().map(|p| std::fs::read(p).expect("reading the profile"));

    let bytes = std::fs::read(&input).unwrap();
    let report = match &out {
        None => mt_core::engine::pdf::check_archive(&bytes, Default::default()).unwrap(),
        Some(path) => {
            let options = mt_core::engine::pdf::ArchiveOptions {
                part: Default::default(),
                icc,
                title: Some("Archived document".into()),
                author: None,
            };
            let (converted, report) = mt_core::engine::pdf::to_archive(&bytes, &options).unwrap();
            std::fs::write(path, &converted).unwrap();
            report
        }
    };

    println!("{}  {}", input, report.part);
    println!("  conformant (by these checks): {}", report.conformant);
    for a in &report.applied {
        println!("  applied: {a}");
    }
    for b in &report.blockers {
        println!("  BLOCKER [{}] {}", b.rule, b.detail);
    }
}
