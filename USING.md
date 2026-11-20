# Using Kitbox

The short version, for the person at the keyboard rather than the one reading
the source. Everything here happens on your own machine; nothing is uploaded,
and there is no account.

## The thing it was built for

A form says *"photograph, JPG, between 20 and 50 KB"*. Open **Compress image**,
drop the photo in, choose **Between**, type `20` and `50`, press Compress. You
get a file inside that range and a note saying what it cost — "97% visually
identical" — rather than five exports and a guess.

If the form also pins the pixels ("200 × 230"), open **Form presets** instead,
find the one that matches, and press **Fix it for this form**. It crops, sizes
and compresses in one go, then checks its own work and shows you the list:

```
Format  JPG            wants JPG
Size    47 KB          wants 20 KB to 50 KB
Pixels  200×230 px     wants 200×230 px
```

The presets are starting points, not official rules. Boards change them. If
yours disagrees, press **Edit these numbers** and it stays edited.

## When you have a pile of files

**Batch** does one job to a whole folder — compress to a size, resize, convert,
or auto-fix. A file that cannot be read is reported by name and the rest carry
on.

**Recipes** are several steps saved as one: *crop to 200 × 230, then land it
between 20 and 50 KB*. Build it once, then drop files on it forever.

## Without opening the app at all

Three ways in, all off until you switch them on in **Settings**:

* **Right-click a file in Explorer** → Kitbox → a size. The result appears in
  your output folder and the window comes up afterwards showing what it did.
  (On Windows 11 it sits under "Show more options".)
* **Copy a picture, press Ctrl + Alt + K.** The compressed *file* lands on your
  clipboard, ready to paste into an upload box.
* **Watch a folder.** Anything your scanner or phone drops into it gets
  compressed. Only new arrivals; the originals are never touched.

## Where things go

Finished files land in `Pictures\Kitbox`, and nothing is ever overwritten — a
second run of the same file becomes `photo-kitbox-2.jpg`.

**History** keeps every file and the settings that made it, so the next time a
form asks for the same thing you can take the finished file instead of making
it again. Click a thumbnail to see where a file came from. Press **Run this
again** to apply a past job's settings to a new file.

It also means Kitbox is holding copies of your documents. Three ways to deal
with that:

* **Forget this** on any entry removes it and Kitbox's copy. The finished file
  in your output folder is yours and stays.
* **Tidy up** clears entries whose file has gone, and stored copies nothing
  points at.
* **Let history forget** (Settings) drops the old ones on a schedule. It shows
  what it would take before you turn it on, and never touches a pinned entry.

## Things it will tell you rather than fake

* A size it cannot reach is reported, with what it managed and what would help
  — not quietly missed.
* A file padded to clear a form's *minimum* says so: part of it is filler.
* Redaction removes the text and then checks it is gone. What was removed is
  never written to history.
* Blurring faces reports how many it found, which is not a promise it found
  them all. Look at the result.
* Upscaling invents detail. It says so.

## If something goes wrong

A screen that fails says which screen and offers to try again — the rest of the
app keeps working. The file you were working on is untouched: everything
happens on a copy.

The models for OCR, background removal, upscaling and face blurring are
downloaded once, on first use, and the app tells you the size and source before
it fetches anything. That and **HTML to PDF** (which fetches the page you ask
for) are the only times Kitbox uses the network.
