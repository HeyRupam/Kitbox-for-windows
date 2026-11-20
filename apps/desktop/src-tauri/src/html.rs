//! Turning a web page into a PDF.
//!
//! This is the one tool that cannot live in `mt-core`. Laying out HTML means a
//! browser engine, and there is already one in the app: WebView2, which ships
//! with Tauri and has `PrintToPdf` built in. Reimplementing CSS layout to keep
//! the rule about platform-free code would produce something far worse than
//! the engine sitting right there.
//!
//! # The window the page is rendered in
//!
//! A page is loaded into its own hidden window labelled [`PRINT_WINDOW`], not
//! the app's. That matters: Tauri grants a window the capabilities its label
//! is listed under, and `capabilities/default.json` lists only `main`. So the
//! page being converted — which may be a site on the internet, running its own
//! JavaScript — can see no Kitbox command at all. Rendering untrusted HTML in
//! a window wired to the app's own commands would be a hole big enough to
//! drive a file-deletion through.
//!
//! # Waiting
//!
//! The page has to finish loading before it is printed, and "finished" is not
//! a single moment on a modern page: the load event fires, then fonts swap and
//! deferred scripts fill things in. So the printing waits for the load event
//! and then a short settle, and both have a timeout — a page that never
//! finishes loading has to fail with a sentence rather than hang the app.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// The label of the window pages are rendered in.
///
/// Deliberately not `main`: the capability file grants permissions by label,
/// so this one has none.
pub const PRINT_WINDOW: &str = "kitbox-print";

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Source {
    /// Raw HTML, written to a temporary file and loaded from there.
    Html(String),
    /// A local `.html` file, so its own stylesheets and images resolve.
    File(String),
    /// A page on the web. The only part of Kitbox that fetches one.
    Url(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSetup {
    /// a4, letter, legal — or the width and height below when "custom".
    pub size: String,
    pub landscape: bool,
    /// Inches, all four edges.
    pub margin: f64,
    /// Print the page's background colours and images.
    pub backgrounds: bool,
    /// 0.1–2.0.
    pub scale: f64,
    /// Add the page's title and URL in the margins.
    pub header_and_footer: bool,
    /// Extra milliseconds to wait after the load event, for fonts and
    /// late-running scripts.
    pub settle_ms: u64,
}

impl Default for PageSetup {
    fn default() -> Self {
        PageSetup {
            size: "a4".into(),
            landscape: false,
            margin: 0.4,
            backgrounds: true,
            scale: 1.0,
            header_and_footer: false,
            settle_ms: 500,
        }
    }
}

impl PageSetup {
    /// Width and height in inches, as WebView2 wants them.
    fn inches(&self) -> (f64, f64) {
        let (w, h) = match self.size.to_ascii_lowercase().as_str() {
            "letter" => (8.5, 11.0),
            "legal" => (8.5, 14.0),
            "a3" => (11.69, 16.54),
            "a5" => (5.83, 8.27),
            _ => (8.27, 11.69),
        };
        if self.landscape {
            (h, w)
        } else {
            (w, h)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HtmlReport {
    pub pages: u32,
    pub source: String,
    pub warnings: Vec<String>,
    pub note: String,
}

pub const RENDERED_NOT_SAVED: &str =
    "This is a picture of the page as it looked when it loaded. Anything that needed a login, \
     appeared after scrolling, or arrived later from a script may be missing, and nothing \
     interactive survives.";

/// How long to wait for a page to load before giving up.
const LOAD_TIMEOUT: Duration = Duration::from_secs(45);
/// How long WebView2 gets to produce the PDF once the page is ready.
const PRINT_TIMEOUT: Duration = Duration::from_secs(60);

/// Open the rendering window on `url` and wait until the page has settled.
///
/// Shared by both outputs: the loading, the waiting and the timeout are the
/// same whether the page is going to be printed or photographed, and two
/// copies of that would drift.
fn open_page(
    app: &AppHandle,
    url: WebviewUrl,
    described: &str,
    settle_ms: u64,
    size: (f64, f64),
) -> Result<tauri::WebviewWindow, String> {
    // Any leftover window from a previous run, closed before making another.
    if let Some(existing) = app.get_webview_window(PRINT_WINDOW) {
        let _ = existing.close();
    }

    let (loaded_tx, loaded_rx) = mpsc::channel::<()>();
    let window = WebviewWindowBuilder::new(app, PRINT_WINDOW, url)
        .title("Kitbox — rendering")
        .visible(false)
        // A definite size, or the layout depends on whatever the window
        // happened to be and two runs of the same page differ.
        .inner_size(size.0, size.1)
        .on_page_load(move |_, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = loaded_tx.send(());
            }
        })
        .build()
        .map_err(|e| format!("could not open a window to render in: {e}"))?;

    if loaded_rx.recv_timeout(LOAD_TIMEOUT).is_err() {
        let _ = window.close();
        return Err(format!(
            "{described} didn't finish loading within {} seconds",
            LOAD_TIMEOUT.as_secs()
        ));
    }

    // Fonts swap and deferred scripts run after the load event.
    std::thread::sleep(Duration::from_millis(settle_ms.min(10_000)));
    Ok(window)
}

/// Render `source` and return the PDF bytes.
pub fn to_pdf(app: &AppHandle, source: &Source, setup: &PageSetup) -> Result<Rendered, String> {
    if !(0.1..=2.0).contains(&setup.scale) {
        return Err("the scale has to be between 0.1 and 2".into());
    }
    if !(0.0..=3.0).contains(&setup.margin) {
        return Err("the margin has to be between 0 and 3 inches".into());
    }

    let temp = tempfile::Builder::new()
        .prefix("kitbox-html-")
        .tempdir()
        .map_err(|e| format!("could not make a working folder: {e}"))?;

    let (url, described) = resolve(source, temp.path())?;
    let output = temp.path().join("out.pdf");

    let window = open_page(app, url, &described, setup.settle_ms, (1200.0, 1600.0))?;

    let mut warnings = Vec::new();
    let result = print(&window, &output, setup);
    let _ = window.close();
    result?;

    let bytes = std::fs::read(&output)
        .map_err(|e| format!("the PDF was printed but couldn't be read back: {e}"))?;
    if bytes.is_empty() {
        return Err("the renderer produced an empty file".into());
    }

    let pages = mt_core::engine::pdf::page_count(&bytes).unwrap_or(0);
    if pages == 0 {
        warnings.push("The result has no pages — the document may have been blank.".into());
    }

    Ok(Rendered {
        bytes,
        report: HtmlReport {
            pages,
            source: described,
            warnings,
            note: RENDERED_NOT_SAVED.to_string(),
        },
    })
}

/// The PDF and what happened while making it.
pub struct Rendered {
    pub bytes: Vec<u8>,
    pub report: HtmlReport,
}

/// How a page is photographed.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotSetup {
    /// Width of the viewport the page is laid out in, in pixels.
    pub width: u32,
    /// Capture the whole page rather than just the first screenful.
    pub full_page: bool,
    /// Height to use when `full_page` is off.
    pub height: u32,
    /// PNG keeps text crisp; JPEG is smaller for a photo-heavy page.
    pub png: bool,
    pub settle_ms: u64,
}

impl Default for ShotSetup {
    fn default() -> Self {
        ShotSetup { width: 1280, full_page: true, height: 900, png: true, settle_ms: 600 }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotReport {
    pub width: u32,
    pub height: u32,
    pub source: String,
    pub warnings: Vec<String>,
    pub note: String,
}

pub struct Shot {
    pub bytes: Vec<u8>,
    pub report: ShotReport,
}

/// Tallest page that will be captured in one piece.
///
/// The whole page has to exist as a window for the capture to see it, and a
/// window of some tens of thousands of pixels stops being composited.
const MAX_SHOT_HEIGHT: u32 = 8_000;

/// Photograph `source`.
pub fn to_image(app: &AppHandle, source: &Source, setup: &ShotSetup) -> Result<Shot, String> {
    if !(200..=5_000).contains(&setup.width) {
        return Err("the width has to be between 200 and 5000 pixels".into());
    }
    if !(200..=MAX_SHOT_HEIGHT).contains(&setup.height) {
        return Err(format!("the height has to be between 200 and {MAX_SHOT_HEIGHT} pixels"));
    }

    let temp = tempfile::Builder::new()
        .prefix("kitbox-shot-")
        .tempdir()
        .map_err(|e| format!("could not make a working folder: {e}"))?;
    let (url, described) = resolve(source, temp.path())?;
    let output = temp.path().join(if setup.png { "out.png" } else { "out.jpg" });

    let window = open_page(
        app,
        url,
        &described,
        setup.settle_ms,
        (setup.width as f64, setup.height as f64),
    )?;

    let mut warnings = Vec::new();
    let mut height = setup.height;
    if setup.full_page {
        match measure_height(&window) {
            Ok(measured) if measured > 0 => {
                height = measured.min(MAX_SHOT_HEIGHT);
                if measured > MAX_SHOT_HEIGHT {
                    warnings.push(format!(
                        "The page is {measured} pixels tall; only the first {MAX_SHOT_HEIGHT} \
                         were captured. Past that a window stops being drawn."
                    ));
                }
            }
            Ok(_) => warnings
                .push("The page didn't report a height, so the window's own was used.".into()),
            Err(e) => warnings.push(format!(
                "Couldn't measure the page ({e}), so the window's own height was used."
            )),
        }

        if height != setup.height {
            // The capture sees the window, so the window has to be the page.
            if let Err(e) =
                window.set_size(tauri::LogicalSize::new(setup.width as f64, height as f64))
            {
                warnings.push(format!("The window wouldn't resize ({e}), so the shot is short."));
                height = setup.height;
            } else {
                // Re-layout and repaint at the new size before the shutter.
                std::thread::sleep(Duration::from_millis(350));
            }
        }
    }

    let result = capture(&window, &output, setup.png);
    let _ = window.close();
    result?;

    let bytes = std::fs::read(&output)
        .map_err(|e| format!("the picture was taken but couldn't be read back: {e}"))?;
    if bytes.is_empty() {
        return Err("the renderer produced an empty picture".into());
    }
    let (shot_w, shot_h) = image::load_from_memory(&bytes)
        .map(|i| (i.width(), i.height()))
        .unwrap_or((setup.width, height));

    Ok(Shot {
        bytes,
        report: ShotReport {
            width: shot_w,
            height: shot_h,
            source: described,
            warnings,
            note: RENDERED_NOT_SAVED.to_string(),
        },
    })
}

/// Turn the source into something the webview can open.
fn resolve(source: &Source, work: &std::path::Path) -> Result<(WebviewUrl, String), String> {
    match source {
        Source::Html(html) => {
            if html.trim().is_empty() {
                return Err("there's no HTML to render".into());
            }
            // Written to a file rather than a data: URL — those have length
            // limits and a different origin, which breaks relative links and
            // some fonts.
            let path = work.join("page.html");
            std::fs::write(&path, html)
                .map_err(|e| format!("could not write the page to render: {e}"))?;
            Ok((WebviewUrl::External(file_url(&path)?), "the pasted HTML".into()))
        }
        Source::File(path) => {
            let path = PathBuf::from(path);
            if !path.exists() {
                return Err(format!("{} isn't there", path.display()));
            }
            let shown = path.display().to_string();
            Ok((WebviewUrl::External(file_url(&path)?), shown))
        }
        Source::Url(url) => {
            let parsed: tauri::Url =
                url.parse().map_err(|_| format!("'{url}' isn't a web address"))?;
            // http and https only: a file:// or custom scheme typed into the
            // address box is a way to reach things the user didn't mean to.
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err("only http and https addresses can be opened".into());
            }
            Ok((WebviewUrl::External(parsed), url.clone()))
        }
    }
}

fn file_url(path: &std::path::Path) -> Result<tauri::Url, String> {
    tauri::Url::from_file_path(path)
        .map_err(|_| format!("{} isn't a path that can be opened", path.display()))
}

/* ---------- the part that talks to WebView2 ---------- */

#[cfg(windows)]
fn print(
    window: &tauri::WebviewWindow,
    output: &std::path::Path,
    setup: &PageSetup,
) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Environment6, ICoreWebView2_2, ICoreWebView2_7,
        COREWEBVIEW2_PRINT_ORIENTATION_LANDSCAPE, COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT,
    };
    use webview2_com::PrintToPdfCompletedHandler;
    use windows::core::{Interface, HSTRING, PCWSTR};

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let path = output.to_path_buf();
    let setup = setup.clone();

    window
        .with_webview(move |webview| {
            // Runs on the UI thread, which is where these calls have to
            // happen.
            let report = |outcome: Result<(), String>| {
                let _ = tx.send(outcome);
            };

            let controller = webview.controller();
            let core = match unsafe { controller.CoreWebView2() } {
                Ok(core) => core,
                Err(e) => return report(Err(format!("no webview to print: {e}"))),
            };

            let printable: ICoreWebView2_7 =
                match core.cast() {
                    Ok(v) => v,
                    Err(_) => return report(Err(
                        "this version of WebView2 can't print to PDF — updating Microsoft Edge \
                         WebView2 should fix it"
                            .into(),
                    )),
                };

            // Print settings come from the environment, not the webview.
            let settings = (|| -> windows::core::Result<_> {
                let two: ICoreWebView2_2 = core.cast()?;
                let environment = unsafe { two.Environment()? };
                let six: ICoreWebView2Environment6 = environment.cast()?;
                unsafe { six.CreatePrintSettings() }
            })();
            let settings = match settings {
                Ok(s) => s,
                Err(e) => return report(Err(format!("could not set up printing: {e}"))),
            };

            let (width, height) = setup.inches();
            let applied = unsafe {
                settings
                    .SetOrientation(if setup.landscape {
                        COREWEBVIEW2_PRINT_ORIENTATION_LANDSCAPE
                    } else {
                        COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT
                    })
                    .and_then(|_| settings.SetPageWidth(width))
                    .and_then(|_| settings.SetPageHeight(height))
                    .and_then(|_| settings.SetMarginTop(setup.margin))
                    .and_then(|_| settings.SetMarginBottom(setup.margin))
                    .and_then(|_| settings.SetMarginLeft(setup.margin))
                    .and_then(|_| settings.SetMarginRight(setup.margin))
                    .and_then(|_| settings.SetScaleFactor(setup.scale))
                    .and_then(|_| settings.SetShouldPrintBackgrounds(setup.backgrounds))
                    .and_then(|_| settings.SetShouldPrintHeaderAndFooter(setup.header_and_footer))
            };
            if let Err(e) = applied {
                return report(Err(format!("those page settings were refused: {e}")));
            }

            let wide = HSTRING::from(path.as_os_str());
            let done = tx.clone();
            let handler = PrintToPdfCompletedHandler::create(Box::new(move |result, success| {
                let outcome = match (result, success) {
                    (Ok(()), true) => Ok(()),
                    (Ok(()), false) => Err("the renderer declined to print the page".into()),
                    (Err(e), _) => Err(format!("printing failed: {e}")),
                };
                let _ = done.send(outcome);
                Ok(())
            }));

            if let Err(e) =
                unsafe { printable.PrintToPdf(PCWSTR(wide.as_ptr()), &settings, &handler) }
            {
                report(Err(format!("printing couldn't be started: {e}")));
            }
            // Success is reported by the handler.
        })
        .map_err(|e| format!("could not reach the renderer: {e}"))?;

    match rx.recv_timeout(PRINT_TIMEOUT) {
        Ok(outcome) => outcome,
        Err(_) => {
            Err(format!("the renderer didn't finish within {} seconds", PRINT_TIMEOUT.as_secs()))
        }
    }
}

#[cfg(not(windows))]
fn print(
    _window: &tauri::WebviewWindow,
    _output: &std::path::Path,
    _setup: &PageSetup,
) -> Result<(), String> {
    // The rest of this module is portable; only the printing is not. Said
    // plainly here so a future port finds one place to fill in rather than a
    // silent wrong answer.
    Err("rendering a page to PDF is only built for Windows so far".into())
}

/// Ask the page how tall it really is.
///
/// Through WebView2's own `ExecuteScript` rather than Tauri's `eval`: eval
/// returns nothing, and the alternative — an IPC channel back to the app —
/// is exactly what this window is deliberately denied.
#[cfg(windows)]
fn measure_height(window: &tauri::WebviewWindow) -> Result<u32, String> {
    use webview2_com::ExecuteScriptCompletedHandler;
    use windows::core::{HSTRING, PCWSTR};

    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    window
        .with_webview(move |webview| {
            let core = match unsafe { webview.controller().CoreWebView2() } {
                Ok(core) => core,
                Err(e) => {
                    let _ = tx.send(Err(format!("no webview: {e}")));
                    return;
                }
            };
            // The tallest of the usual measures: a page can be laid out so any
            // one of them is short.
            let script = HSTRING::from(
                "Math.max(document.body ? document.body.scrollHeight : 0,                  document.documentElement.scrollHeight,                  document.documentElement.offsetHeight)",
            );
            let done = tx.clone();
            let handler = ExecuteScriptCompletedHandler::create(Box::new(move |result, value| {
                let _ = done.send(match result {
                    Ok(()) => Ok(value),
                    Err(e) => Err(format!("{e}")),
                });
                Ok(())
            }));
            if let Err(e) = unsafe { core.ExecuteScript(PCWSTR(script.as_ptr()), &handler) } {
                let _ = tx.send(Err(format!("{e}")));
            }
        })
        .map_err(|e| format!("could not reach the renderer: {e}"))?;

    let raw = rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "the page didn't answer".to_string())??;
    // The result arrives as JSON, so a number comes back as "1234".
    raw.trim()
        .trim_matches('"')
        .parse::<f64>()
        .map(|v| v.ceil().max(0.0) as u32)
        .map_err(|_| format!("the page gave '{raw}' for its height"))
}

#[cfg(not(windows))]
fn measure_height(_window: &tauri::WebviewWindow) -> Result<u32, String> {
    Err("only built for Windows so far".into())
}

/// Photograph what the webview is showing, straight to a file.
#[cfg(windows)]
fn capture(
    window: &tauri::WebviewWindow,
    output: &std::path::Path,
    png: bool,
) -> Result<(), String> {
    use webview2_com::CapturePreviewCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_JPEG,
        COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    };
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{STGM_CREATE, STGM_READWRITE};
    use windows::Win32::UI::Shell::SHCreateStreamOnFileEx;

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let path = output.to_path_buf();

    window
        .with_webview(move |webview| {
            let report = |outcome: Result<(), String>| {
                let _ = tx.send(outcome);
            };
            let core = match unsafe { webview.controller().CoreWebView2() } {
                Ok(core) => core,
                Err(e) => return report(Err(format!("no webview to photograph: {e}"))),
            };

            // Written straight into a file stream, so the picture never has to
            // be carried through memory twice.
            let wide = HSTRING::from(path.as_os_str());
            let stream = match unsafe {
                SHCreateStreamOnFileEx(&wide, (STGM_CREATE | STGM_READWRITE).0, 0, true, None)
            } {
                Ok(stream) => stream,
                Err(e) => return report(Err(format!("could not open {}: {e}", path.display()))),
            };

            let done = tx.clone();
            let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
                let _ = done.send(match result {
                    Ok(()) => Ok(()),
                    Err(e) => Err(format!("the picture couldn't be taken: {e}")),
                });
                Ok(())
            }));

            let format = if png {
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG
            } else {
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_JPEG
            };
            if let Err(e) = unsafe { core.CapturePreview(format, &stream, &handler) } {
                report(Err(format!("the picture couldn't be started: {e}")));
            }
        })
        .map_err(|e| format!("could not reach the renderer: {e}"))?;

    match rx.recv_timeout(PRINT_TIMEOUT) {
        Ok(outcome) => outcome,
        Err(_) => Err("the renderer didn't finish the picture in time".into()),
    }
}

#[cfg(not(windows))]
fn capture(
    _window: &tauri::WebviewWindow,
    _output: &std::path::Path,
    _png: bool,
) -> Result<(), String> {
    Err("photographing a page is only built for Windows so far".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silly_shot_size_is_refused() {
        // Checked before a window is opened, so these need no app handle.
        for width in [0u32, 10, 9000] {
            let setup = ShotSetup { width, ..Default::default() };
            assert!(!(200..=5_000).contains(&setup.width), "{width} should be out of range");
        }
        let tall = ShotSetup { height: 50_000, ..Default::default() };
        assert!(tall.height > MAX_SHOT_HEIGHT, "a window that tall is not drawn");
    }

    #[test]
    fn a_shot_defaults_to_the_whole_page_as_a_png() {
        let setup = ShotSetup::default();
        assert!(setup.full_page, "a screenshot of the first screenful only is rarely wanted");
        assert!(setup.png, "text stays crisp in PNG and smears in JPEG");
        assert_eq!(setup.width, 1280);
    }

    #[test]
    fn the_shot_settings_cross_as_the_ui_writes_them() {
        let setup: ShotSetup = serde_json::from_str(
            r#"{ "width": 1280, "fullPage": false, "height": 900, "png": false, "settleMs": 600 }"#,
        )
        .expect("shot setup");
        assert!(!setup.full_page);
        assert!(!setup.png);
        assert_eq!(setup.settle_ms, 600);
    }

    #[test]
    fn page_sizes_turn_round_when_landscape() {
        let portrait = PageSetup { size: "a4".into(), ..Default::default() };
        let (w, h) = portrait.inches();
        assert!(h > w, "A4 portrait is taller than it is wide");

        let landscape = PageSetup { landscape: true, ..portrait.clone() };
        let (lw, lh) = landscape.inches();
        assert_eq!((lw, lh), (h, w), "turning it round swaps the two");
    }

    #[test]
    fn letter_and_legal_are_not_the_same_page() {
        let letter = PageSetup { size: "letter".into(), ..Default::default() }.inches();
        let legal = PageSetup { size: "legal".into(), ..Default::default() }.inches();
        assert_eq!(letter.0, legal.0, "same width");
        assert!(legal.1 > letter.1, "legal is longer");
    }

    #[test]
    fn an_unknown_size_falls_back_to_a4_rather_than_zero() {
        let odd = PageSetup { size: "postcard".into(), ..Default::default() }.inches();
        let a4 = PageSetup { size: "a4".into(), ..Default::default() }.inches();
        assert_eq!(odd, a4);
    }

    #[test]
    fn only_web_addresses_are_opened() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["file:///C:/Windows/System32/drivers/etc/hosts", "javascript:alert(1)"] {
            let error = resolve(&Source::Url(bad.into()), dir.path()).unwrap_err();
            assert!(
                error.contains("http") || error.contains("web address"),
                "{bad} should be refused: {error}"
            );
        }
        assert!(resolve(&Source::Url("https://example.com".into()), dir.path()).is_ok());
    }

    #[test]
    fn empty_html_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve(&Source::Html("   ".into()), dir.path()).is_err());
    }

    #[test]
    fn a_missing_file_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let error = resolve(&Source::File("C:/nope/nothing.html".into()), dir.path()).unwrap_err();
        assert!(error.contains("isn't there"), "{error}");
    }

    #[test]
    fn pasted_html_is_written_somewhere_the_webview_can_open() {
        let dir = tempfile::tempdir().unwrap();
        let (url, described) = resolve(&Source::Html("<h1>hello</h1>".into()), dir.path()).unwrap();
        assert_eq!(described, "the pasted HTML");
        match url {
            WebviewUrl::External(url) => assert_eq!(url.scheme(), "file"),
            other => panic!("expected a file URL, got {other:?}"),
        }
        assert!(dir.path().join("page.html").exists());
    }

    #[test]
    fn the_rendering_window_is_not_the_one_with_permissions() {
        // capabilities/default.json grants its permissions to "main" alone, so
        // a page loaded here can reach no Kitbox command. If this label ever
        // becomes "main", untrusted pages gain the run of the app.
        assert_ne!(PRINT_WINDOW, "main");
    }
}
