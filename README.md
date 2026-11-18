# Kitbox

An offline PDF and image toolkit for Windows. Everything runs on your own
machine — no uploads, no file-size caps, no daily limits.

The feature it exists for: **compress to a size, not a guess.** Type the range a
form demands — "between 50 and 100 KB" — and Kitbox encodes the file repeatedly
until the result lands inside it, then tells you exactly what that cost.

## What works today

All 39 tools are built. Every one is in the app, on the command line, and
covered by tests.

| | |
|---|---|
| Image compress to a target size | ✅ |
| **PDF compress to a target size** | ✅ |
| **PDF size inspector** — where the bytes went | ✅ |
| **Page tools** — merge, split, extract, remove, reorder, rotate | ✅ |
| **Images → PDF**, and PDF → JPG for scans | ✅ |
| **Resize, crop, rotate, convert** | ✅ |
| **Protect and unlock** — passwords and permissions | ✅ |
| **Watermark** — text or a logo, on images and PDFs | ✅ |
| **Page numbers** — formats, positions, rotated pages | ✅ |
| **Crop PDF** — margins, percent, or measured from the scan | ✅ |
| **Repair PDF** — recover a damaged file, honestly | ✅ |
| **PDF to Markdown** — text with inferred structure | ✅ |
| **Compare PDF** — what changed, line by line | ✅ |
| **Scan to PDF** — find the page, flatten it, clean it up | ✅ |
| **Sign PDF** — cut a signature out of a photo and place it | ✅ |
| **Redact PDF** — delete content, then check it's gone | ✅ |
| **PDF forms** — fill the fields, and lock them in | ✅ |
| **Edit PDF** — text, highlights, shapes, freehand | ✅ |
| **OCR** — read a scan, keep the page, add a hidden text layer | ✅ |
| **HTML to PDF** — a web page or your own markup, rendered | ✅ |
| **HTML to image** — the same page, photographed whole | ✅ |
| **Photo editor** — tone and colour, with live preview | ✅ |
| **PDF/A** — convert what can be, and name what can't | ✅ |
| **Captions** — the heavy white-on-black look | ✅ |
| **Remove background** — subject kept, backdrop your choice | ✅ |
| **Upscale** — resample instantly, or reconstruct with a model | ✅ |
| **Blur faces** — find them, cover them, show what was found | ✅ |
| **Batch** — one job, a whole folder of files | ✅ |
| **Explorer right-click** — compress without opening the app | ✅ opt-in, Windows |
| **Clipboard key** — copy a picture, press a key, paste a file | ✅ opt-in, Windows |
| **Watched folders** — anything that lands gets compressed | ✅ opt-in |
| **Recipes** — several steps, saved and re-run as one | ✅ |
| **Form presets** — check a file against a form, and fix it | ✅ editable JSON on disk |
| Local history with **lineage shown**, filters and thumbnails | ✅ every tool |
| **Forget** — remove an entry, its copy and its thumbnail | ✅ with a tidy-up sweep |
| **Run a past job again** on a new file | ✅ from History |
| Command line | ✅ every tool |

Measured on a real 1.3 MB, 4-page document: **93 KB** in 8 encodes, about 3.5
seconds, every page intact and the text still text.

## One list, not two

Each tool tile carries a `status`, and the app routes a tool only if a screen
exists for it. Those were separate lists, and they drifted: five finished tools
sat on the home screen wearing a "Soon" badge and could not be opened, because
only the routing list got updated when each one landed. The routing list is now
derived from the tiles' own status, so there is one place to change.

## A screen can stop without taking the window

React's rule about a render error is blunt: it unmounts the whole tree. The
first time that happened here — a results list reading a field that serde had
named differently — the app didn't show an error. It showed a white window,
which looks like a dead app rather than a wrong one, and tells you nothing.

Every screen now renders inside an error boundary, with a second one around the
whole shell. A screen that cannot draw is replaced by a panel that names it,
says the files are untouched, shows what went wrong and offers three ways on:
try the screen again, go back to the tool list, or copy the details. The nav
rail sits outside the boundary, so there is always somewhere to click.

Verified by breaking a screen on purpose in the running app — during render,
where it matters, not in a click handler — then leaving it by the panel's own
button and reopening it working.

## Running it

Tauri picks where the window loads from by a cargo feature: without
`custom-protocol` the app is a *dev* build and points at the dev server, with
it the frontend is baked into the binary. The usual default is off, which means
a plain `cargo build --release` produces an app that launches, sits there
looking healthy, and shows a connection-refused page. Nothing about the build
says so.

So `custom-protocol` is a **default feature** here, and `npm run app` turns it
back off for hot reload. A test asserts a built app isn't a dev build, because
"the process started" is not the same as "the app works" — which is exactly the
mistake that put that error page on screen for a while.


```bash
npm install && npm --prefix ui install
```

```bash
npm run app
```

That builds the Rust core and opens the desktop window. For UI work alone,
`npm run dev` serves the interface at <http://localhost:1420> with a canvas-based
engine standing in for the Rust one, so every screen works in a plain browser.

```bash
npm run app:build
```

produces `target/release/bundle/nsis/Kitbox_<version>_x64-setup.exe` — about
7.5 MB, a per-user install needing no administrator.

The first run of this downloads NSIS and one helper DLL from the Tauri
project's own releases, verifies their hashes and caches them. That is build
tooling rather than anything the app ships with.

A note for anyone changing the layout: `beforeBuildCommand` and
`beforeDevCommand` run from the directory the Tauri CLI was invoked in, which
the npm scripts make `apps/desktop` — *not* `src-tauri`, where
`frontendDist` is resolved from. The two are relative to different places and
looked consistent while both were wrong. Neither had ever been run until the
first real bundle build, which failed instantly on a path three levels up
instead of two.

## The command line

```bash
cargo run --release -p mt-cli -- compress photo.jpg --target 50-100kb
```

```bash
cargo run --release -p mt-cli -- compress scan.jpg --target 20-50kb --exact 200x230 --format jpg
```

```bash
cargo run --release -p mt-cli -- compress report.pdf --target 50-100kb
```

```bash
cargo run --release -p mt-cli -- inspect report.pdf
```

`inspect` prints the byte breakdown and every embedded image with its real
resolution, so you can see what you're fighting before you compress.

The whole form-filing chain, end to end:

```bash
cargo run --release -p mt-cli -- to-pdf page1.jpg page2.jpg -o documents.pdf
```

```bash
cargo run --release -p mt-cli -- compress documents.pdf --target "under 200kb"
```

Page and image operations:

```bash
cargo run --release -p mt-cli -- pages report.pdf --keep 2-5 --rotate 90
```

```bash
cargo run --release -p mt-cli -- resize photo.jpg --fill 200x230
```

`merge`, `split`, `to-images`, `crop`, `rotate` and `convert` work the same way;
`pages` with no options lists what's in the document.

`--bias keep-sharp` holds resolution and spends quality; `--bias shrink-pixels`
does the opposite. For PDFs, `--min-dpi` sets the resolution floor and
`--grayscale` lets colour go as a last resort. `kitbox history` lists what
you've made.

## Layout

```
crates/mt-core      engines, the size solver, quality scoring — no platform deps
crates/mt-store     SQLite index over a content-addressed blob store
crates/mt-cli       the engines without a window; also the test harness
apps/desktop        Tauri 2 shell: window, paths, history wiring
ui                  React + TypeScript, Kitbox design system
design              the reference screens the UI is built from
```

`mt-core` never touches Tauri, the filesystem or a subprocess. Bytes in, bytes
out. That is what lets the same engine run on Android later without a rewrite.

## How the solver works

Each engine offers the solver a *ladder* of encoder settings ordered from
smallest output to largest. The solver:

1. tries the best setting first — if the file already fits, nothing is lost;
2. tries the worst — if even that overshoots, it says so and suggests what to
   change, rather than returning something mangled;
3. fits a curve through those two points, jumps to the predicted setting, and
   bisects what's left;
4. stops as soon as a result uses most of the window, since rungs are spaced a
   few percent apart and resolving further costs encodes but no visible quality;
5. scores the winner with SSIM so the result card can say what it cost.

Typically four to seven real encodes. A 5.8 MB photo into a 50–100 KB window
takes about 300 ms.

When a form wants a *minimum* size and the image is smaller even at full
quality, the file is padded with data every decoder ignores — a JPEG comment
segment, a PNG ancillary chunk — and the result is reported as padded, not
passed off as ordinary compression.

## How PDFs differ

"Make this PDF smaller" means different work depending on what the PDF is, so
compression runs in stages:

0. **Inspect.** Attribute every byte — images, fonts, page content, metadata —
   and work out each image's *real* resolution by walking the page content
   streams and tracking the transformation matrix. An image isn't 300 DPI
   because it has a lot of pixels; it's 300 DPI because of how big it's drawn.
1. **Lossless.** Repack streams into object and cross-reference streams, strip
   metadata, drop orphans, and point every copy of a repeated image at one
   object. Nothing visible changes. If that reaches the target, it stops there.
2. **Downsample.** Re-encode the embedded images at a lower resolution and
   quality, chosen by the same solver. Text is never touched, so the document
   stays selectable, searchable and printable.

Images it won't re-encode are listed with the reason: JPEG 2000 and JBIG2 data
can't be put back faithfully, and indexed or spot-colour images shift colour if
pushed through RGB. CMYK is converted properly rather than reinterpreted.

**Kitbox will not rasterise your pages.** Flattening a document into pictures of
words always hits a smaller number, and silently destroys text selection, search
and screen-reader access. When downsampling can't reach the target, it says so
and suggests what to change instead.

## Page tools

Merge, split, extract, remove, reorder and rotate are one operation underneath —
build a document from the pages you name, in the order you name them, turned the
way you ask. Writing it once means a fix to the page-tree handling fixes every
tool at once.

Two things that quietly break naive implementations are handled explicitly.
Pages inherit `MediaBox`, `Resources` and `Rotate` from parent nodes, so those
are copied down before the tree is flattened — otherwise merged pages lose their
size. And once pages move, bookmarks, tagging and named destinations point at
pages that may not exist, so they're dropped and the result *says* they were.

Page thumbnails work without a PDF renderer: for a scanned page, the image is
already in the file and is simply recovered. A text page reports that it has no
preview rather than showing a blank, and `to-images` says the same.

## Comparing

The naive version of this compares page 1 to page 1, page 2 to page 2, and so
on — so inserting one page at the front reports every page after it as changed.
That is the thing people complain about, so pages are aligned first and lines
within them second, using the same diff for both.

Whitespace and case are normalised away, because they are not changes anyone
wants reported. Scanned pages carry no text; rather than calling them identical
because neither had anything to say, their images are compared and the result
is labelled as a visual comparison.

## Rendering a web page

This is the one tool that cannot live in `mt-core`. Laying out HTML means a
browser engine, and there is already one in the app: WebView2, which ships with
Tauri and has `PrintToPdf` built in. Reimplementing CSS layout to keep the
no-platform-dependencies rule would produce something far worse than the engine
sitting right there, so this part is frankly Windows-only, and the one
non-Windows branch says so rather than returning a silently wrong answer.

### A picture instead of a document

The same window, the same wait, a different shutter: `CapturePreview` rather
than `PrintToPdf`. The output's extension decides which, so `--render page.pdf`
prints and `--render page.png` photographs — one flag, no mode to remember.

Capturing the *whole* page rather than one screenful means the page has to
exist as a window first, because the capture photographs the window. The height
comes from the page itself through WebView2's `ExecuteScript` — not Tauri's
`eval`, which returns nothing, and not an IPC channel, which is exactly what
this window is deliberately denied — then the window is resized and repainted
before the shutter. Verified on a 120-row page: 6052 pixels tall with the last
row and the closing banner both present, which is the check that matters, since
the obvious failure is a tall image whose lower half is blank.

Past 8000 pixels a window stops being composited, so that is the limit, and
going over it is reported rather than silently truncated.

Two things worth knowing. The picture comes out in *physical* pixels, so a 1280
layout on a 125%-scaled display gives a 1600-wide image — sharper, not wrong.
And a page shorter than the viewport is captured at viewport height with blank
space below, because that is what `scrollHeight` reports; trimming to the
content would cut off anything positioned outside it.

### The window the page loads in

A page is rendered in its own hidden window labelled `kitbox-print` — not the
app's. Tauri grants a window the capabilities its label is listed under, and
`capabilities/default.json` lists only `main`. So the page being converted,
which may be a site on the internet running its own JavaScript, can see no
Kitbox command at all. Rendering untrusted HTML in a window wired to the app's
own tools would be a hole big enough to drive a file-deletion through. There is
a test asserting that label never becomes `main`.

Only `http` and `https` are accepted. A `file://` or custom scheme typed into
the address box is a way to reach things nobody meant to.

### Waiting

"Finished loading" is not a single moment on a modern page: the load event
fires, then fonts swap and deferred scripts fill things in. So it waits for the
load event and then a short settle, and both have timeouts — a page that never
finishes has to fail with a sentence rather than hang the app.

### It has a command line after all

Rendering needs a browser engine and an event loop, and the `kitbox` CLI has
neither. Rather than leave the tool clickable-only — unscriptable, and
impossible to test without a person in front of it — the app itself takes the
job:

```bash
kitbox-desktop --render https://example.com out.pdf
```

That is how it was verified: a local invoice with tables, borders and a tinted
box came out with its CSS intact, and `https://example.com` rendered with its
background and its Arabic and CJK font fallback.

### What you get

A picture of the page as it looked when it loaded. Anything behind a login,
further down a lazy-loading list, or added later by a script may be missing,
and nothing interactive survives. The screen says so before you press the
button, and so does every report.

## Several steps, saved as one

Every tool screen does one job. A recipe is for the job someone actually has:
*crop it to 200 × 230, land it between 20 and 50 KB* is one task to the person
and two operations to the engine. Saved once, it becomes a drop — and it runs
over as many files as you hand it.

```
Bank form photo
  1. Crop and fit to 200×230
  2. Compress to 20–50 KB
```

Run on a 2.5 MB passport photo and a 1.2 MB selfie, both came out at exactly
200 × 230 and 20 KB, with the trace shown step by step:

```
Crop and fit to 200×230 (1.2 MB → 13 KB)
Compress to 20 KB to 50 KB (13 KB → 20 KB), padded with filler to clear the minimum
```

That last clause matters. The photos came out under the 20 KB floor the form
demands, so they were padded to clear it — the numbers alone would just look
like the file got bigger for no reason.

### Compressing goes last, and the app says so

Every step after a compress re-encodes the file, which throws away the size
that was just solved for. So a recipe with compression anywhere but the end is
**refused when you save it**, with the reason:

> compressing has to be the last step — anything after it would undo the size

Not quietly obeyed, and not silently reordered. The same check refuses a
recipe that compresses twice, and one with no steps at all.

### What a step can need

A watermark step needs a font, and the engine reads no files, so the shell
supplies one. If there is no usable font the recipe **refuses** rather than
skipping the step — a file that looks right and is missing its stamp is worse
than an error.

A PDF can only be compressed here, so a recipe with picture steps in it turns a
document away by name instead of failing halfway through.

Recipes live in `recipes.json` beside the form presets, editable by hand like
everything else in that folder.

## A folder that looks after itself

Point Kitbox at the folder your scanner writes to, or the one your phone syncs
into, and anything that lands there is compressed to the size you set. Add it
in Settings, pick a size, and that is the whole setup.

Three rules shape it, all of them about not doing damage:

* **Arrivals only.** Pointing at a folder with five hundred photos in it does
  not compress five hundred photos. What is already there when watching starts
  is noted and left alone — and still left alone after a restart.
* **Never the original.** Results go to the output folder like every other
  tool. A watcher that edited files in place could destroy a scanner's only
  copy.
* **Wait until it stops moving.** This is the one that bites. A file exists
  from its first byte and may be written for seconds afterwards, so taking it
  on sight gets half a photo. A file is only read once its size has stopped
  changing *and* it can actually be opened — on Windows a file can have its
  final size and still be locked by whatever made it.

Tested by behaving like a scanner: an 811 KB photo written into a watched
folder in eight chunks over four seconds came out as a complete 1440 × 1080
picture at 48 KB. Dropped whole, two photos were compressed and a `.txt` beside
them ignored. After a restart, the three files already sitting there stayed
untouched and only the new arrival was taken.

Names that mean "still arriving" — `.crdownload`, `.part`, `.tmp` and friends —
are skipped, as is anything Kitbox itself produced, so a watch pointed at the
output folder cannot chase its own tail.

## Copy, press, paste

The right-click menu covers files on disk. The clipboard key covers how images
actually arrive the rest of the time: a screenshot, a crop from a web page,
something pasted out of a chat — pictures with no path at all.

Turn it on in Settings, copy a picture, press **Ctrl + Alt + K**, and the
compressed **file** is on your clipboard, ready to paste into an upload box.
Windows puts the path straight into the File name field, and Explorer pastes it
as a copy.

A file, not a picture — and that is the whole point. A clipboard picture is an
uncompressed bitmap, so handing one back would throw the compression away the
moment anything read it. Measured on a 1600 × 1200 screenshot: **7.7 MB on the
clipboard → a 97 KB file**, in about a second, with the app's window never
appearing.

A toast says what happened, because a key that seems to do nothing is worse
than no key — including when it can't: press it with text on the clipboard and
it says so and leaves the clipboard exactly as it was.

The key is off until you ask for it, and three combinations are offered rather
than a free-text box, because a program that quietly takes a key combination
away from the whole machine is not a good guest. If the one you pick is already
spoken for, saving fails with what has it instead of pretending to listen.

## Pinning

A pinned entry survives a tidy-up. That is all it does, and it is the only
thing that stops one: a sweep skips it even when its file has gone.

It existed in the database from the start and had no button, which meant the
protection was real and unreachable — forget and tidy both honoured a flag
nothing could set. Now it is the small badge on each history card. Forgetting a
pinned entry on purpose still works; that is an explicit answer to an explicit
question, and the confirmation says so.

## Where a file came from

A file is rarely a thing on its own. It is the latest step of something: the
scan, then the copy that got under 300 KB, then the one that got under 90, then
the 300 px version for the form. Every job has recorded its input, its output
and the settings between them since the first version — and none of it was ever
on screen.

Clicking a thumbnail in History now opens that chain:

```
photo.bmp 5.8 MB  →  photo-kitbox.jpg 284 KB     compress · 321 ms
photo-kitbox.jpg  →  photo-kitbox-kitbox.jpg     compress · 238 ms
...-kitbox.jpg    →  ...-kitbox-kitbox.jpg       resized · 26 ms
```

Each step opens to show the settings it used, as JSON, because "which quality
did I use last March" is exactly the question a gallery should answer. What was
made *from* a file is listed too.

A step whose file has since been forgotten still appears, named as gone. The
step happened; hiding it would make the chain a lie.

Filter chips sit above the grid — everything, pinned, and one per tool that
actually appears, rather than a row of twenty chips for an empty history.

### Two things this turned up

**Entries made by the command line could not be re-run.** The app records a
`bias` with every compress and the command line doesn't, so the replay reader
refused the whole recipe over one missing field. A recipe that lacks a setting
the tool has since grown now replays with the default, which is what older
entries deserve.

**The command line recorded almost nothing.** `compress` wrote to history and
twenty-seven other commands did not: a file made on the command line never
appeared in the gallery, and any chain running through it had a hole where that
step should be. (The first count of this was five — that was what a quick look
found, not what was there.)

Every one of them records now. Writing and recording are a single call,
`deliver`, so the two cannot drift apart again, and a test walks the source
asserting that every command which writes a file also records it — the same
guard the app keeps over its own commands. `protect` and `unlock` are the
deliberate exceptions, as they are in the app: keeping a copy of a document
someone just encrypted, or the open version of one they unlocked, is a privacy
problem rather than a feature.

A four-step chain built entirely on the command line — compress, resize, make a
PDF, number the pages — now shows up in the gallery as one story, including the
step that began with several files and so has nothing before it.

## Proving the promise

The app rests on one claim: you state the size and the file lands inside it.
Everything else — the forms, the recipes, the right-click menu — ends in the
same solver, so that is the thing to test hardest.

**Two hundred pairs nobody chose.** A seeded generator makes files (crops,
phone photos, scans) and targets (plausible, awkward, and a few nothing could
ever reach), and checks the invariant every time:

> a `Hit` inside the range, a `Padded` inside the range, or an `Infeasible`
> that says so — never a success that is outside what was asked for.

The last run: 139 hit, 49 padded into range, 12 honestly refused, none wrong.
Each failure would print the seed that produced it, so a bad pair can be
reproduced exactly rather than hunted. The full sweep takes about three and a
half minutes and is `#[ignore]`d; a 24-pair version runs with the suite.

**A benchmark, separately.** The sweep proves the solver doesn't lie; the
corpus says how good it is — hit rate, encodes spent, how much of the picture
survives, and how long it takes, per kind of file:

```
cargo test -p mt-core --release --test corpus -- --ignored --nocapture
```

Its corpus is synthetic, and the output says so. The plan asks for fifty real
PDFs and photographs, which would be better — real cameras and scanners have
quirks no generator invents. Until there is a corpus that can live in the
repository without someone's documents in it, this is what there is, and it
catches a regression.

### What the benchmark found on its first run

A 37 KB crop, asked to come in **under 100 KB**, came back as 68 KB. It already
fitted, and the top of the quality ladder re-encoded it into something larger —
the one way a compressor can genuinely insult you.

There was even a test called `returns_the_untouched_file_when_it_already_fits`.
It asserted one encode, which is what the code did; the name said what it
should do. Nobody had noticed the two disagreeing.

A file that already meets its target now comes back **byte for byte**, with no
encode at all. Where metadata is being stripped it still is — JPEG and PNG both
keep that in whole blocks that can be dropped without touching a pixel, so the
file gets smaller and the picture is identical. "Already small enough" stops
being an answer the moment the request means more than a size: a different
format, or fixed dimensions, and it works as before.

On the corpus that turned three re-encodes into untouched files and took the
average from 4.3 encodes to 3.8.

**CI is written down even though nothing runs it.** There is no remote yet, so
`.github/workflows/ci.yml` does nothing today. It is in the repository because
the checks that matter should be written before they are needed: formatting,
clippy with warnings denied, the suite on Windows, the interface's types and
build, and the full solver sweep on main and overnight.

## Forgetting

History keeps a copy of every file that goes in and comes out, which is what
makes lineage work and what makes a gallery worth having. It also means Kitbox
is holding copies of your documents, so getting rid of one has to be a real
thing it can do.

**Forget this** on any entry removes the row, its job, Kitbox's copy of the
bytes and the thumbnail — a thumbnail left behind is a picture of the thing
someone asked to be rid of. It asks once first, and says plainly that the
finished file in your output folder is yours and stays where it is: the only
copy being deleted is the one Kitbox made.

**Tidy up** handles the two kinds of clutter that accumulate on their own:

* **Entries whose file has gone.** Files get moved, renamed and deleted outside
  the app. An entry pointing at nothing is worse than no entry — it offers to
  show you a file that isn't there.
* **Stored copies nothing points at.** Inputs are kept so a chain stays whole.
  Once the jobs referring to one are forgotten, its copy is unreachable: not in
  the gallery, not in any lineage, not findable — just disk space. Clearing 38
  test entries this way freed 18.9 MB that nothing in the interface could even
  show you.

Pinned entries are left alone by both, because pinning is how you say the
stored copy should be kept whatever happens to the original.

## The same thing you did last time

Every job already recorded the settings that produced it. History now reads
them back: pick a past entry, choose a new file, and it runs again — "compress
another file to 20 KB–50 KB", "fix another photo for IBPS", "split another
statement the same way".

The settings go nowhere near the screen. Only the entry's id crosses; the
recipe is read from history on the engine's side, so what runs is what was
recorded rather than something the interface reconstructed from a summary.

### Some of them can't, and say so

The interesting half of this feature is the refusals. Several tools never
recorded what they did, on purpose:

| | |
|---|---|
| **Redact** | What a redaction removed is never written down — that is the point of it. |
| **PDF forms** | The answers typed into a form aren't recorded, only which fields were filled. |
| **Sign** | Where a signature went, and the signature itself, aren't kept. |
| **Protect / unlock** | Passwords are never kept, which is deliberate. |
| **Merge, compare, JPG→PDF** | These work on several files at once, so there's nothing to hand them here. |
| **Watermark with a logo** | The logo's bytes would sit in every history row, so only the fact of it was recorded. |

Those entries show the reason in place of the button, rather than a button that
fails when you press it. A recipe this build doesn't recognise is treated the
same way — no button, and it says so — because a history row written by a
future version shouldn't break the screen that shows it.

The model-backed tools (OCR, background removal, upscale, blur faces) point you
at the tool instead: they need a model that the tool loads when you open it.

### A replay is a job like any other

It writes to the same folder, lands in history under the same tool with the
same recipe, and can be replayed again in turn — so a chain of "the same thing
again" stays readable rather than becoming a special case.

Pages are the one place a replay can fail honestly: a job that numbered pages
8–9 of a long document, run on a three-page one, says *"the pages this job used
(8-9) don't fit this document"* rather than quietly numbering nothing.

## What the form actually asked for

A portal that rejects your application rarely says why. So a preset is not a
shortcut for typing two numbers — it is the form's rules written down, and a
file measured against them.

**Checking reports everything it compared**, passed or failed, with what the
form wants beside what the file is:

```
This file would be rejected
  Format  JPG            wants JPG
  Size    3.3 MB         wants 20 KB to 50 KB
  Pixels  3000×2000 px   wants 200×230 px
```

That list is the point. A verdict with no workings is no use when you are
holding it against the page in front of you, and a checklist you can read is
how you find out the app and the form disagree about what "100 KB" means.

**Fixing** crops to the stated shape, solves for the size, writes the
resolution if the form asks for one, and then **checks its own work** and shows
the same list again. If the result still doesn't conform it says so and what is
left, rather than handing back a file that will be rejected.

Three things it will tell you that other tools don't:

* **"padded with filler to clear the minimum"** — when a form demands *at
  least* 20 KB and the best encode comes out at 14, the file is padded
  losslessly to clear the floor. That works, and it is a surprising thing to
  hand someone silently.
* **A resolution the file never states is "unknown", not "wrong".** Most photos
  say nothing about DPI and most forms never look. Failing a file for that
  would send people resampling a photo that was fine.
* **Pixels can be a floor, not a fixed size.** "At least 350 × 350" leaves the
  solver free to spend pixels on reaching the size; "exactly 200 × 230" doesn't,
  and then quality is the only lever.

### They are files, because the rules change

Boards revise requirements between cycles, two portals for the same exam
disagree, and the person filling the form can see the real numbers while this
app cannot. So the presets live in a folder you own:

```
%LOCALAPPDATA%\com.kitbox.desktop\presets\
  yours.json               what you changed or added
  bundled-reference.json   a copy of the built-in set, to crib from
  README.txt               the field reference
```

The bundled set is compiled into the engine and always current. Anything in
your folder overrides or adds to it **by id**, so updating Kitbox improves the
built-in numbers without touching your edits, and deleting an entry puts the
bundled one back — there is no state to repair.

Editing a preset in the app writes the same file you would edit by hand. There
is one copy of the bundled set in the whole repository, imported by both the
engine and the UI, because the first thing a second copy does is disagree with
the first about a number.

### Being honest about a folder people type in

Two failures found by using it rather than reading it:

**A valid preset file was rejected.** Written in PowerShell with
`Out-File -Encoding utf8`, which — like Notepad — starts the file with a byte
order mark. JSON parsers reject that on the first character, so a correctly
typed preset came back as *"this isn't a preset file"*: a true sentence about
the bytes and a useless one to read. Files with a BOM, and files saved as
UTF-16, now load.

**A file that can't be read is named, not skipped.** One bad file reports
itself on the screen with the parser's complaint, and every other preset still
loads. A preset that silently vanishes is a bug report nobody can write. And a
single preset typed without the surrounding brackets — the commonest mistake —
is told exactly that, instead of "invalid type: map".

A `yours.json` that doesn't parse is never overwritten, either. Someone's edits
are in there, and losing them to save one preset would be the worst thing this
code could do.

## Without opening anything

The fastest version of this job skips the app entirely. Turn on the right-click
menu in Settings and Explorer gets:

```
Kitbox ▸ Compress to 20–50 KB
        Compress to 50–100 KB
        Compress to under 100 KB
        Open in Kitbox
```

on pictures, and the sizes a document actually needs (100 / 200 / 500 KB) on
PDFs. Pick one and the file is in your output folder. The window comes up
afterwards showing what it did, file by file — the work is finished before you
see it.

It appears on picture and PDF files only. Adding Kitbox to the menu for every
file on the machine would be rude and useless.

### Forty files, one window

Explorer runs a right-click verb **once per selected file**, so forty photos
means forty launches. Forty copies of the app, forty windows, forty history
rows — or one of each, which is what this does: the first instance takes the
job, the rest hand their file to it and exit, and after a short pause the whole
selection runs as a single batch with one list of results.

### Leaving nothing behind

The menu is written under `HKEY_CURRENT_USER`: no administrator, nothing
outside your own account, and turning it off deletes exactly the keys it added
— verified by checking all ten are gone afterwards.

Two ways it could have rotted, both handled:

* **Uninstalling.** The keys live in your registry, not in the program folder,
  so an uninstall would leave a menu pointing at a program that no longer
  exists. The installer carries a hook that removes them.
* **Moving the app.** Each entry names an exe. On every start, an installed
  menu that points somewhere else is rewritten to point at the copy that is
  running.

## A folder at a time

The single-file screens answer "what do I do with this file". Batch answers the
other question, the one a form actually asks: *forty photographs, every one
under 100 KB, by Friday.*

Compress, resize, convert and auto-fix each run over the whole pile. Compress is
the one that also takes PDFs; the others are pictures only, and a PDF among them
is reported by name and skipped rather than mangled.

```bash
kitbox batch ./scans --op compress --target 50-100kb --out ./ready
```

```bash
kitbox batch ./photos --op resize --longest 1600 --out ./small
```

A folder argument is read one level deep — deliberately not recursively, since
nobody enjoys discovering that typing `.` walked their home directory.

### One bad file costs you that file

The whole point of a batch is that you stop watching it. So a file that cannot
be read is charged against its own name and the run carries on:

```
beach.jpg  214 KB -> 95 KB  (97.3% identical, 7 attempts)
holiday-big.jpg  618 KB -> 93 KB  (89.8% identical, 9 attempts)
invoice.jpg  failed: invoice.jpg isn't a picture or a PDF — whatever its name
             says, the bytes aren't either
report.pdf  1.3 MB -> 93 KB  (92.5% identical, 8 attempts)
screenshot.jpg  303 KB -> 82 KB  (98.1% identical, 5 attempts)

4 done, 0 fell short, 1 failed
```

Three outcomes, not two: **done**, **fell short** — produced, but the target was
not reachable, so you got the smallest honest version instead of an error — and
**failed**, with the reason. Stop mid-run and everything after the current file
is listed as not reached, by name.

### Why it sends one file at a time

The obvious shape is to hand the whole pile over in one call. Measured instead
of assumed: bytes cross into the engine as JSON, where a 4 MB photograph
measures about **14 MB**. Forty of those is half a gigabyte in a single string —
the exact size of job this screen exists for would have been the one that broke
it. One file per call keeps the peak at one file, lets each result appear as it
lands, and makes Stop take effect at the next file rather than at the end.

Stopping also stops the sending: the run already knows every file's name, so the
ones it never reached are accounted for without shipping their bytes across to
be handed straight back.

### Resizing is not a request to re-encode

The encoder's "keep the format" rule turns an opaque PNG into a JPEG, which is
the right trade when you are chasing a size and the wrong one when the only
thing you asked for was pixels — a screenshot would come back with soft text
nobody asked to pay for. So compress may change format; resize and auto-fix
keep it.

### One row in the gallery, not forty

History records the run — the operation, how many files, how many landed — not
each file separately. Every output is still stored and findable; it's the job
row that is folded into one, so a batch doesn't bury everything else you did
that week.

## Captions

The look is a specific one and getting it wrong is obvious: heavy upright
capitals, white, thick black outline, centred top and bottom. The outline is
the part that matters — white text alone vanishes into a pale sky, and a drop
shadow reads as something else entirely.

It is made by spreading the letters' own coverage outwards and filling that
with black, then laying the white on top. Spreading in a circle rather than a
square, because a square spread gives the letters visible corners where there
should be none. Stroking each glyph would be more faithful and needs outline
geometry the rasteriser doesn't keep; at these sizes the result is the same.

Long captions wrap on their own and never reach the edge — there is a test
that checks the outermost columns of the finished picture are untouched. The
alternative arrangement puts the top line on a white band *above* the picture
instead of over it, growing the canvas rather than covering anything, and the
colours reverse there so the words don't disappear into the white.

The font comes from the machine, as every font in this crate does: `mt-core`
opens no files. Impact is what this look is made of and is on every Windows
install, with heavier faces as fallbacks.

## Archiving

PDF/A is a PDF that carries everything it needs to render: its fonts, a
definite meaning for its colours, no scripts, nothing linked from outside, no
encryption whose keys are lost.

### It checks; it does not certify

Conformance is a long specification and the last word belongs to a validator
built for the job. This applies the fixes it can apply correctly, checks the
rules it can check honestly, and names every one it cannot meet. `conformant:
true` means *every rule listed here passed* — and the note says so in every
report, next to the result rather than in small print.

Claiming more would be the worst kind of wrong: a file that says PDF/A on the
tin, is accepted into an archive, and fails validation in ten years when
nobody remembers why.

### Fonts decide it

A document using Helvetica without embedding it depends on the reader owning
Helvetica, which is precisely the dependency PDF/A exists to remove. Kitbox
ships no font files, so it cannot fix that — it reports which fonts, and that
the fix lies elsewhere.

Which splits the world neatly, and in a useful direction: **a scan has no fonts
at all and converts cleanly**, and scans are most of what anyone archives. A
text document written with the standard fonts does not. Worth knowing that this
includes text Kitbox itself adds — watermarks, page numbers, signatures, OCR
layers and form appearances all use base-14 Helvetica.

### The colour profile comes from your machine

PDF/A wants an output intent with an ICC profile. Rather than ship one — with
the licensing and the staleness that invites — the desktop app reads Windows'
own `sRGB Color Space Profile.icm` and hands the bytes to `mt-core`, which
reads no files by rule. Same arrangement as the models. Where there isn't one,
that becomes a reported blocker rather than a guess.

### What it fixes

Scripts, embedded files, open actions and automatic actions removed; a file
identifier derived from the content, so re-archiving the same document twice
gives the same identifier rather than a new one; XMP metadata stating the part
and conformance, written uncompressed because a reader is meant to find it
without decoding anything; and an `Info` dictionary rewritten to agree with the
XMP, since a validator treats disagreement between them as a fault.

## Tone and colour

The geometry tools crop, rotate and resize; nothing touched tone until now.
This is for the picture everyone actually has: a document shot indoors that
came out dim, flat and yellow.

Adjustments are not commutative, so the order is fixed and written down —
levels, colour, exposure, contrast, saturation, greyscale, then sharpening
last on a settled picture. Everything in between runs in floating point;
rounding to bytes seven times produces banding that shows in a sheet of paper
long before anyone can name why.

### Two bugs, both found by looking

**Grey-world was the wrong assumption.** The textbook automatic white balance
averages the whole picture and assumes it should be neutral. A document on a
wooden desk genuinely *is* mostly brown, so grey-world read the desk as a cast
and swung the picture conspicuously blue. The brightest pixels are a far better
clue, and for this app's usual subject they are the paper — so whatever colour
the paper came out is the colour of the light. Where there is no highlight to
read, nothing is changed rather than guessed.

**Then it was still blue**, because the correction was being applied twice.
Stretching each channel to its own range is itself a colour correction — that
is most of what levels does to a cast — but the colour reading was taken from
the *original*. Levels are now applied first, in full, and the light is read
from the result: the gain on the test photograph went from 1.35 on blue (the
clamp, i.e. as far as it could go) to 1.04. There is a test that fails if any
channel's gain drifts more than 0.2 from neutral when both are on together.

### Preview

The live preview runs on a downscaled copy so the sliders stay responsive.
That is honest here in a way it would not be for a geometry tool: every
adjustment is per-pixel and scale-free, and the two automatic ones read the
picture's own histogram, which a fair downscale preserves. What you see is what
gets saved, at full size.

### What it cannot do

Lifting a very dark photo lifts its noise, and a JPEG's blocking comes up too.
There is no detail hiding in there — only what the sensor recorded — and the
report says so when it has pushed hard enough for that to show.

## Covering faces

The detector is [UltraFace](https://github.com/onnx/models) RFB-320 (MIT),
through the same `rten` runtime as everything else: 1.2 MB, about seven
milliseconds a picture. Its boxes come out already decoded, which is why it was
picked over YuNet — anchor decoding across three strides is a lot of arithmetic
to get silently wrong in a tool whose failures are invisible.

### A missed face is the whole failure

Everything else here fails visibly: a bad crop looks wrong, a bad OCR reads
wrong. This one fails *invisibly and in the direction that matters*. You share
the photo believing the faces are covered and one of them isn't, because it was
in profile or small or half behind a shoulder.

So it is built to be checked rather than trusted. Every face found is outlined
on the result, the count is stated, and the note says a count is not a promise.
A face with no box round it is the thing to notice before pressing save.

### Three covers, and only one of them destroys

A solid box destroys the pixels underneath. Blur and pixelation disguise them,
and both have been undone before — which the screen says whenever one is
chosen.

The pixelation default was wrong on the first pass, and only looking at the
output caught it: ten blocks across a face left all three test faces perfectly
recognisable, which defeats the entire tool. The range is now 12 blocks down to
4, and there is a test that counts how many distinct colours survive in the
covered area.

Boxes are grown by a margin before covering, because a box drawn tightly round
the features leaves the ears, chin and hairline — plenty to recognise someone
by. Blur radius and mosaic block size both scale with the face, so a small face
isn't left with four readable blocks and a large one isn't left readable at all.

### Letterboxing

The network takes a fixed 320×240. Squashing a portrait into 4:3 distorts every
face in it and detection starts failing, so the picture is letterboxed into the
frame on grey — grey rather than black, because a hard black border reads as an
edge and the detector finds things along it — and the boxes are mapped back out
afterwards.

## Making a picture bigger

Two ways, because they answer different questions and one tool pretending to
do both would mislead.

**Sharpen** is Lanczos plus an unsharp mask. Instant, no size limit, nothing to
download, and it invents nothing — it resamples what is already there. For a
photo slightly too small for a form, this is the right answer.

**Reconstruct** is Real-ESRGAN, which has learned what detail tends to look
like and puts some back. That is the point and the catch: it is *plausible*
detail, not recovered detail. Excellent on a portrait or a logo; misleading on
anything read as evidence — a face, a plate, a serial number. Every report says
so.

### The cost, measured not guessed

About **125 seconds per megapixel of input** on this machine at 16 threads,
measured across four tile sizes before any code was written around it. A
200×230 form photo is six seconds. A 12-megapixel phone photo would be
twenty-five minutes.

That is not a thing to let someone start by accident, so the model path refuses
above about 1.1 megapixels — with the arithmetic in the message, not just a no:

```
this picture is 3.0 megapixels, and reconstruction would take about 6 minutes.
Crop it, or shrink it below 1.1 megapixels first — or use Sharpen, which has
no limit.
```

The screen quotes the wait for *your* picture before the button is pressed.

### Tiling

Output is sixteen times the pixels of the input, so a whole-image pass would
need a tensor far larger than the picture. The work goes tile by tile, each
grown by a margin that is thrown away afterwards — a model does its worst work
at the edge of what it can see, and without the margin every boundary leaves a
visible seam. There is a test that measures the column difference at a tile
boundary against its neighbours and fails if it stands out.

### Picking the model

Same discipline as the background remover: probe first. The ONNX Model Zoo's
2016 super-resolution model is public but weak; the Qualcomm and several
lighter Real-ESRGAN conversions are gated or gone. `baker76/upscale-models`
serves a 70 MB RRDBNet x4 that `rten` loads and runs correctly — and feeding it
ImageNet normalisation produces values outside 0..1 that clip to a washed-out
picture, so it takes plain 0..1 RGB, confirmed by checking the output range
rather than by assuming.

70 MB is much the largest download in Kitbox, and it buys only half of this
tool. Sharpen works on a fresh install with nothing fetched.

## Removing a background

The case this exists for: a form wants a passport photo on plain white or
blue, and what you have is a photo against a wall.

The model is [U²-Net](https://github.com/xuebinqin/U-2-Net) (Apache 2.0, the
4.6 MB `u2netp` variant), run through `rten` — the same pure-Rust runtime the
OCR tool already uses. So this adds no new inference dependency, and `mt-core`
keeps its rule about having no platform dependencies. About 200 ms a photo.

Picking the model was the first job, and it was settled by measurement rather
than reputation: `u2netp` loads and runs correctly, while the larger `silueta`
returns a degenerate mask — every value exactly 1.0 — under the same
preprocessing. That is a runtime-support problem, not a tuning one, so it isn't
shipped. If edge quality disappoints on real photos, the registry below makes
adding another model a two-line change.

### The edge is where a guess shows

The model returns a confidence per pixel, not a decision. Hard-thresholding
that gives the jagged, obviously-cut-out look and destroys hair, so the
confidence goes through a ramp instead: a pixel the model is unsure about comes
out semi-transparent, which is what hair actually is. Two sliders move the ramp
— how wide it is, and where it sits.

A colour backdrop is mixed in here rather than left to whoever saves the file.
A JPEG has no alpha, so flattening later would quietly default to white and
ignore the blue that was asked for.

The honest failure is a photo with no clear subject. The model will still
return something, and it will be wrong — so the report gives the fraction kept
and names the two shapes that mean it went wrong. A flat wall comes back as
"0.4% kept" with a warning, not as a confident empty cutout.

### One registry for every model

Model fetching is no longer OCR's private business. `MODEL_SETS` lists every
downloadable file in one table — id, filename, URL, exact byte count — and the
two commands that check and fetch take the set's name. These are the only two
places in Kitbox that touch the network, so they are listed together rather
than buried in the tools that want them: the size and the source are what the
user is asked to agree to, and both come from there.

Adding the third model-backed tool (Upscale, Blur face) is now a table entry
plus a screen.

## Reading a scan

A scanned page is a photograph. Search, compare, redact and Markdown all look
at it and report, honestly, that there is nothing there. OCR is what puts the
words back.

The engine is [`ocrs`](https://github.com/robertknight/ocrs), which is **pure
Rust**. That matters more than it sounds. Tesseract would mean vendoring two C
libraries for every platform, and `mt-core`'s whole rule is that it has no
platform dependencies so the Android port can reuse it unchanged. Models arrive
as bytes: this module never opens a file or touches the network, and the caller
decides where the bytes came from.

### The page is never changed

The output keeps the scan exactly as it is and adds the recognised words as
**invisible text** (`3 Tr`) positioned over the pixels they were read from.
Nothing about the page looks different — verified side by side.

That is the whole design. OCR is a guess, and a guess that rewrites your
document is dangerous, while a guess that only fills in a hidden index costs
you a bad search result at worst. Every report says so.

Each word is placed and horizontally stretched (`Tz`) so the selectable box
matches the ink it came from; without that a short word highlights as a long
smear. Words on a line carry an explicit trailing space, because every word is
drawn at its own absolute position and nothing in the stream implies a gap —
the first version read back as `Nameofapplicant` and every two-word search
failed.

### What it unlocks

Before and after, same command, same file:

```
$ kitbox redact scan.pdf --text "Photograph"
0 glyphs and 0 images removed on pages

$ kitbox redact scan-searchable.pdf --text "Photograph"
20 glyphs and 1 image removed on page 1
  checked: the content is gone from the finished file
```

The raw scan had nothing to find. After OCR the same redaction removes the
words *and* paints out the pixels under them — because the redactor already
knew to do that, it just needed something to aim at.

### One of the two times Kitbox needs the internet

The models are two files, about 12 MB together. They are fetched once, kept on
the machine, and never needed again. The download is never silent: the size,
the source URL and the destination folder are all on screen before anything is
fetched, because quietly reaching out to the network would be a poor joke in an
app whose main promise is that it doesn't. A short download is rejected rather
than saved, and files land under a `.part` name until complete.

Everything else in Kitbox works with no network at all, OCR included once the
models are there. The other exception is rendering a web page, where typing an
address is itself the request to fetch it.

### Limits

It reads pages that *are* an image, which is what a scan is. A page already
made of text doesn't need OCR and is skipped. A page built from vector drawing
can't be rasterised here — there is no PDF renderer in this crate — and is
reported rather than silently dropped.

The model-backed test is `#[ignore]`d: a suite that quietly needs a 12 MB
download is a suite that fails for the wrong reasons. Run it with
`KITBOX_OCR_MODELS=<dir> cargo test -- --ignored`. The placement maths, the
spacing and the escaping are tested without models, by building recognised
lines by hand.

## Drawing on a page

The fallback for every form that isn't fillable, which is most of them. A
scanned application, a printed invoice with a wrong figure, a contract that
needs a note in the margin: no fields to type into, and all of them need
something written on them.

The marks go into the page's own content rather than becoming PDF annotations.
Annotations are editable later, which sounds better until you consider what
they are — a layer any reader can hide, any tool can strip, and some printers
ignore. A note that silently fails to print is worse than no note. So the marks
become part of the page, and the report says plainly they can't be peeled off.

**Highlighting multiplies rather than covers.** A translucent yellow box
painted over black text greys the text; painted underneath, it vanishes behind
a scan's background image. Multiply blending is what a real highlight
annotation uses and the only thing that works on both: white paper takes the
colour, dark ink stays dark. Verified on a scanned page, where the page itself
is a photograph — the line reads straight through the yellow.

**White-out covers; it does not delete.** Painting over a mistake hides it from
a reader and leaves every character in the file, which is exactly the
black-rectangle trick the redaction tool exists to replace. Every run that uses
one says so and points at Redact.

Text wraps to the box you drew, shapes inset by half their stroke so the
outline doesn't straddle the edge, ellipses are the usual four Bézier curves,
and freehand ink is given in whole-page fractions so a stroke can cross
anything. Marks that share an opacity and blend mode share one graphics state
rather than adding a near-duplicate to the page's resources for each.

## Filling forms

Two traps separate a form that works from one that looks filled in on your
machine and arrives empty.

**A value is not an appearance.** Setting a field's `/V` records what it says;
it draws nothing. Most readers render the *appearance stream* and ignore the
value, so a file filled by setting `/V` alone shows up blank in half the
world's PDF viewers and on every printer. The honest-looking shortcut is
`/NeedAppearances true`, which asks the reader to draw it — Acrobat obliges,
plenty of others don't, no printer does. So the appearance streams are built
here, with `/DA` parsed for the font, size and colour the form asked for,
auto-sizing when it says `0 Tf`, wrapping in multi-line boxes, and honouring
`/Q` alignment.

**A checkbox's "on" is not `/Yes`.** It is whatever key the widget's `/AP`
dictionary uses — `/Yes` often, `/On` sometimes, `/1`, `/Ja` or something the
author invented the rest of the time. Writing `/Yes` into a field whose on-state
is `/1` leaves a box that is neither on nor off and renders empty. So "yes" is
translated into whatever this particular form calls on, and a value the field
cannot take is refused with the list of what it does take, rather than written
anyway.

The test fixture deliberately uses `/On` for its checkbox and `post`/`email`
for its radio group, so the easy wrong assumption fails loudly.

Unknown field names are reported rather than ignored — a typo in a field name
is otherwise completely silent. So are read-only fields, values cut short by
`/MaxLen`, and required fields still empty when the file is written. A password
field is drawn as dots: rendering the characters into the page would defeat the
field entirely.

### Flattening

Drawing the values into the pages and removing the form is what "submit a
filled PDF" nearly always means — nothing can be edited, and nothing can be
cleared by a reader that decides to regenerate appearances.

The obvious implementation keeps each appearance as a form XObject and `Do`s
it. That renders correctly and makes the values **unextractable**, because text
extraction reads a page's own content stream and does not descend into
XObjects. A flattened form whose answers can't be copied, searched or read by a
screen reader is a worse document than the one that went in. So the operators
are copied into the page instead, with each appearance's resources carried
along under prefixed names so the form's `/Helv` can't collide with a different
`/Helv` the page already had.

One more thing has to happen first: a field can hold a value and have no
appearance at all — forms built around `/NeedAppearances` ship none, and a
value set by another tool usually has none either. Flattening those drops the
answer off the page while cheerfully reporting success. Every field with a
value gets an appearance generated before anything is drawn. The test checks a
read-only field that was never touched still appears in the flattened text, for
exactly that reason.

Flattening is one-way, and the report says so.

## Redacting

Most tools that offer this draw a black rectangle over the page and call it
done. The text is still underneath: select it, copy it, or run any extractor
and it comes back out. People have leaked witness names and lost cases this
way. A black rectangle is a decoration, not a redaction.

So this deletes the glyphs from the content stream. A glyph is removed by
replacing it with the negative kerning adjustment equal to its own width, so
everything after it stays exactly where it was — delete the glyph and nothing
else and the rest of the line slides left, which is both ugly and a tell.

Three things make the difference between this and the rectangle:

**The old drawing is deleted, not orphaned.** Pointing the page at a rewritten
stream and leaving the original object behind passes every obvious check: the
page looks right and extraction finds nothing, while the text sits in the file
for anyone with a hex editor. The first version of this module did exactly
that. The verification pass caught it, which is the argument for having one.

**On a scan, the picture of the word goes too.** A scanned contract is an image
with an invisible OCR text layer on top. Delete the text layer alone and the
word is unextractable while the *photograph* of it sits untouched under the
bar — peel the bar off and read it. So the pixels under every bar, including
bars that came from matching text, are painted out of the embedded image.

**It checks itself.** Every run re-reads its own output and goes looking for
what was meant to be gone — first by extracting each page's text, then by
sweeping every object in the file with its streams decompressed, because
extraction only reads the streams a page points at. The result travels in the
report, the CLI exits non-zero when it fails, and the screen leads with it.
"Redacted" is a thing a tool asserts; "I looked again and it wasn't there" is a
thing it can show.

Metadata, bookmarks, embedded files, form data and annotations go as well — a
title or a bookmark routinely repeats the thing being removed.

### What it can't do

Vector drawings — ruled lines, shapes, a signature stored as paths — are
covered by the bar but their geometry stays in the file. Removing arbitrary
path geometry safely is a different problem, and quietly pretending otherwise
is the exact failure this tool exists to avoid, so every report says so.

Fonts whose glyphs aren't single bytes can't be cut up letter by letter. There
the unit becomes the whole run of text: all of it goes or none does, and the
report says that happened rather than leaving half a secret behind.

## Signing

Two halves, and the first is the one that makes it worth having. You photograph
your signature on paper once; the paper is divided out the same way a scan's
background is, and what's left becomes the **alpha channel** rather than being
pushed to white. The result is a transparent PNG that is kept, so the next form
is one click.

The transparency is the whole point. A signature saved as a white rectangle
covers the line it is meant to sit on, and every form has a line. That means
the PDF side can't use the watermark's image path, which goes through JPEG —
it embeds the colour data and a `/SMask` holding the alpha, both deflated on
the way out.

Alpha comes off a ramp, not a threshold. A hard cutoff turns a pen stroke into
a jagged two-colour shape that reads as a bad fax and crawls when it's scaled
down onto a form. The ramp keeps the soft edge the pen actually made.

Placement is given as fractions of the page rather than points, so a spot
picked on a preview means the same thing on A4 and on Letter — and on a page
whose `/Rotate` says it is displayed sideways, since the same `Frame` the page
numbers use maps displayed coordinates back into page space.

### What it isn't

This draws a picture of a signature. It does not prove who signed, and it does
not make the document tamper-evident. Anyone can take the image out again, and
anyone holding the image can put it on anything.

That is true of every tool that does this, including the ones that charge for
it — it is just rarely said out loud. So it is said here: the module documents
it, every `SignReport` carries it, the CLI prints it, and the screen shows it
above the button rather than after the fact. A legally binding signature needs
PKCS#12 or a DSC token, which is a different mechanism and is not built yet.

## Scanning a photograph

Three steps, each of which can be done badly in a way that is worse than not
doing it: find the page in the frame, flatten it, clean it up.

Finding it needs no vision library. The frame's border is whatever the page is
lying on, so anything differing from it by more than the border's own spread is
page. That mask is then denoised with a majority filter, and the four corners
fall out as the extremes of `x+y` and `x−y`.

The filter is what makes taking the *true* extreme safe, and that matters more
than it sounds. The first version took each corner at a percentile instead, to
stop a speck of dust defining one — and quietly pulled all four corners inward
along their own diagonals, shaving the margins off every scan and leaving the
page still skewed. It looked plausible until the corners were measured against
a photograph whose real corners were known. There is a test holding them within
20px of the truth now.

Flattening is a homography solved from those four corners by direct linear
transform, inverted so each output pixel samples the photograph bilinearly. The
page's width is the average of its two opposite edges, since the near edge is
longer than the far one and the truth is between them. The corners are then
pulled in by 0.6%, because the boundary pixel is part desk and left alone it
shows up as a dark fringe down two sides of the page.

Cleaning up divides out a wide box blur of the image — a background estimate —
rather than applying a contrast curve, which cannot tell a shadow from ink and
darkens both. Dividing removes the shadow down one side of a phone photo and
leaves the writing.

Detection fails on exactly the photographs people take: a white page on a white
table, or a page that already fills the frame. So failure is never silent. The
whole frame is used instead, the report names every page it happened to, and
the preview shows the result before anything is written. A confidently
mis-cropped scan is much worse than one that says it couldn't find the edges.

## Reading a PDF back

A PDF has no paragraphs, headings or lists. It has glyphs at coordinates. So
`markdown.rs` walks the content streams itself rather than taking a flat
string: positions and font sizes are exactly what a flat extraction discards,
and they are the only evidence structure can be inferred from. Headings come
from text set larger than the body size (the commonest size, weighted by how
much text is set in it), paragraphs from vertical gaps, lists from bullets.

It reads an ordinary document well and a multi-column layout badly — reading
top to bottom interleaves columns that were never adjacent. Rather than
silently producing nonsense, it detects pages that look like a layout (many
short lines) and says the order there will be jumbled. A scanned page has no
text at all, and that is reported as needing OCR instead of returned blank.

## Repairing

`lopdf` already does the expensive part — rebuilding a cross-reference table by
scanning the file for indirect objects. What Kitbox adds is everything around
it: trying the gentler approaches first, putting loose pages back into a tree
when the index that held them is gone, and **saying what actually happened**.

A repair tool that reports success on a file it quietly emptied is worse than
one that refuses, because the result gets sent on before anyone notices. So
every result carries findings *and* caveats. Cut a file off at 82% and it
recovers all four pages, while telling you the two whose images were past the
cut are missing them.

## Cropping

Cropping a PDF sets `/CropBox`, which tells readers which part of the page to
show. **The content outside it is still in the file** — select-all picks it up,
and another tool can widen the box again. Every PDF cropper works this way, and
it is fine for trimming scan borders. It is not a way to remove something from a
document, so Kitbox says that on screen before you crop and in every result.

`--auto` measures the page rather than guessing: it decodes the scanned image,
finds the rows and columns that are not background (taken from the corners, so
cream and grey scans work), and maps that back through the rectangle the image
is drawn into. A page made of text and vectors has no image to measure, so it is
reported as skipped instead of cropped to a guess.

## Overlays

Watermarks and page numbers share `overlay.rs`, because the fiddly parts are the
same for both and wrong is quiet rather than loud. The page's own content gets
fenced inside `q`/`Q` — a stream that leaves the graphics state unbalanced,
common in files from older software, would otherwise drag the overlay's
transform with it. Resources are merged under `Kbx`-prefixed names rather than
replaced, which is how a stamping tool turns a scan into a blank page with text
on it.

Page numbers also honour `/Rotate`. A scanned page is often stored upright and
rotated for display; positions are given in *displayed* space and transformed
back, so "bottom right" is the bottom right as the reader sees it and the
number comes out the right way up.

## Watermarks

Text needs no embedded font — readers are required to provide Helvetica, so a
stamp costs a few hundred bytes instead of a font program. The trade is that
the widths have to come from the specification to centre the text, which is why
the standard Helvetica metrics are a table in `watermark.rs`.

The page's own content is wrapped in `q`/`Q` before the mark is drawn. Without
that, a page whose content stream leaves the graphics state unbalanced — common
in files from older software — drags the watermark's transform along with it
and puts the text somewhere unpredictable. Resources are merged rather than
replaced, under `Kbx`-prefixed names, so nothing already on the page is lost.

A logo is embedded once and referenced from every page it lands on.

Images are the harder case: there is no built-in font, so `ab_glyph` rasterises
the glyphs and the font bytes come from the caller. `mt-core` keeps the list of
where fonts usually live but never opens a file — knowing what kind of machine
this is belongs to the shell. Text and logos then share one path: build an RGBA
stamp, scale it to a fraction of the image width, rotate it with bilinear
sampling, composite.

## History

Every tool records what went in, what came out and the settings that connect
them — not just compression. Outputs get a thumbnail where one can be made
(for a PDF, its first page, recovered rather than rendered), so the gallery is
something you can look through rather than a list of filenames.

A source-walking test fails the build if a command writes a file without
recording it, because that is exactly how this regresses: a tool gets added,
the output lands on disk, and the gallery never hears about it.

Protect and unlock are the deliberate exceptions, for the reason below.

## Passwords

A PDF can carry two different passwords. The *open* password is needed to read
it at all; the *owner* password only governs whether you may print, copy or
edit. The second is the one people meet most — a statement that opens fine but
refuses to print — and the Protect/Unlock screen shows which situation you are
in before you do anything.

`protect` writes AES-256 by default, with AES-128 for readers older than about
2010. Leaving the owner password empty falls back to the open password, because
an empty one would let anyone lift the restrictions. Extraction for assistive
technology is always permitted regardless of the copying setting.

`unlock` removes protection from a document you can open: either you supply the
password, or the file opens with an empty one and only permissions were set.
**It does not guess, brute-force or attack passwords.** There is no
candidate-iteration code in it, a file you cannot open stays closed, and the
refusal says so.

Neither tool records anything in local history. Keeping Kitbox's own copy of a
document you just put a password on — or the open version of one you unlocked —
in a store you didn't ask about would be a nasty surprise.

## Testing

```bash
cargo test && npm --prefix ui run typecheck
```

The load-bearing test is `never_returns_an_out_of_range_success`: across a
spread of ladders and targets, a successful result is always inside the
requested range. Everything else in the app rests on that.
