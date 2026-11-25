//! Local history: everything Kitbox reads or writes is kept, so a file prepared
//! once never has to be prepared again.
//!
//! Blobs live in a content-addressed tree keyed by BLAKE3, which means the same
//! bytes stored twice cost one copy — and re-running the same job on the same
//! input is free. A SQLite index carries the metadata, the job that produced
//! each output, and which input it came from, so the gallery can show a file's
//! whole lineage rather than a flat list of leftovers.

mod schema;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no such file in history: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// What the caller knows about a file at the moment it is stored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileMeta {
    pub name: String,
    pub kind: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub pages: Option<u32>,
    /// Where it came from on disk, when it came from disk.
    pub source_path: Option<String>,
    /// Hash of a previously stored thumbnail blob.
    pub thumb_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub hash: String,
    pub size: u64,
    pub name: String,
    pub kind: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub pages: Option<u32>,
    pub source_path: Option<String>,
    pub thumb_hash: Option<String>,
    pub created_at: i64,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: i64,
    pub tool: String,
    /// The recipe as JSON, so the exact same treatment can be replayed later.
    pub recipe: String,
    pub input_hash: String,
    pub output_hash: String,
    /// One line for the UI: "4.8 MB → 87 KB, quality 68".
    pub summary: String,
    pub duration_ms: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewJob {
    pub tool: String,
    pub recipe: String,
    pub input_hash: String,
    pub output_hash: String,
    pub summary: String,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Usage {
    pub files: u64,
    pub bytes: u64,
    pub jobs: u64,
}

pub struct Store {
    root: PathBuf,
    db: Connection,
}

impl Store {
    /// Open (creating if needed) a store rooted at `root`.
    pub fn open(root: impl AsRef<Path>) -> Result<Store> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("blobs"))?;
        let db = Connection::open(root.join("history.db"))?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        db.execute_batch(schema::SCHEMA)?;
        Ok(Store { root, db })
    }

    /// In-memory store for tests.
    pub fn open_temporary(root: impl AsRef<Path>) -> Result<Store> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("blobs"))?;
        let db = Connection::open_in_memory()?;
        db.execute_batch(schema::SCHEMA)?;
        Ok(Store { root, db })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Store bytes and index them. Storing the same bytes twice is a no-op on
    /// disk and returns the existing record, so repeated runs don't accumulate.
    pub fn put(&self, bytes: &[u8], meta: FileMeta) -> Result<FileRecord> {
        let hash = blake3::hash(bytes).to_hex().to_string();
        let path = self.blob_path(&hash);

        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            // Write to a temporary name first so a crash can't leave a blob that
            // is indexed but truncated.
            let tmp = path.with_extension("partial");
            fs::write(&tmp, bytes)?;
            fs::rename(&tmp, &path)?;
        }

        if let Some(existing) = self.get(&hash)? {
            return Ok(existing);
        }

        let now = now_millis();
        self.db.execute(
            "INSERT INTO files (hash, size, name, kind, width, height, pages, source_path, thumb_hash, created_at, pinned)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0)",
            params![
                hash,
                bytes.len() as i64,
                meta.name,
                meta.kind,
                meta.width,
                meta.height,
                meta.pages,
                meta.source_path,
                meta.thumb_hash,
                now,
            ],
        )?;

        Ok(FileRecord {
            hash,
            size: bytes.len() as u64,
            name: meta.name,
            kind: meta.kind,
            width: meta.width,
            height: meta.height,
            pages: meta.pages,
            source_path: meta.source_path,
            thumb_hash: meta.thumb_hash,
            created_at: now,
            pinned: false,
        })
    }

    pub fn read(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.blob_path(hash);
        if !path.exists() {
            return Err(Error::NotFound(hash.to_string()));
        }
        Ok(fs::read(path)?)
    }

    pub fn get(&self, hash: &str) -> Result<Option<FileRecord>> {
        self.db
            .query_row("SELECT * FROM files WHERE hash = ?1", params![hash], row_to_file)
            .optional()
            .map_err(Into::into)
    }

    pub fn record_job(&self, job: NewJob) -> Result<JobRecord> {
        let now = now_millis();
        self.db.execute(
            "INSERT INTO jobs (tool, recipe, input_hash, output_hash, summary, duration_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                job.tool,
                job.recipe,
                job.input_hash,
                job.output_hash,
                job.summary,
                job.duration_ms,
                now
            ],
        )?;
        Ok(JobRecord {
            id: self.db.last_insert_rowid(),
            tool: job.tool,
            recipe: job.recipe,
            input_hash: job.input_hash,
            output_hash: job.output_hash,
            summary: job.summary,
            duration_ms: job.duration_ms,
            created_at: now,
        })
    }

    /// Most recent outputs first — what the gallery and the home strip show.
    pub fn recent_outputs(&self, limit: u32) -> Result<Vec<FileRecord>> {
        let mut stmt = self.db.prepare(
            "SELECT f.* FROM files f
             JOIN jobs j ON j.output_hash = f.hash
             GROUP BY f.hash
             ORDER BY MAX(j.created_at) DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], row_to_file)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Every file this store knows about, newest first.
    ///
    /// The storage panel has to look at all of them rather than a recent page:
    /// the entry worth tidying is usually the one nobody has looked at for a
    /// year, which is exactly the one a recent page leaves out.
    pub fn all_files(&self) -> Result<Vec<FileRecord>> {
        let mut stmt = self.db.prepare("SELECT * FROM files ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], row_to_file)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Files nothing points at any more.
    ///
    /// Kitbox keeps a copy of what goes in as well as what comes out, so that
    /// a chain stays whole. When the jobs that referred to one are forgotten,
    /// its copy is left holding disk space nothing can reach: not in the
    /// gallery, not in any lineage, not findable. That is what this finds.
    ///
    /// A job is not the only thing that can point at a row. Every thumbnail
    /// is stored as a file in its own right, and no job ever mentions one -
    /// so asking about jobs alone called every thumbnail in the store junk,
    /// and the storage panel offered to clear away the very pictures the
    /// gallery was drawing with. Nothing was actually lost, because the blob
    /// is kept while any row still points at it, but the offer was a lie:
    /// it counted the bytes as freed and freed none of them.
    pub fn unreachable(&self) -> Result<Vec<FileRecord>> {
        let mut stmt = self.db.prepare(
            "SELECT * FROM files f
             WHERE NOT EXISTS (SELECT 1 FROM jobs j
                               WHERE j.input_hash = f.hash OR j.output_hash = f.hash)
               AND NOT EXISTS (SELECT 1 FROM files p WHERE p.thumb_hash = f.hash)
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_file)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<FileRecord>> {
        let like = format!("%{query}%");
        let mut stmt = self
            .db
            .prepare("SELECT * FROM files WHERE name LIKE ?1 ORDER BY created_at DESC LIMIT ?2")?;
        let rows = stmt.query_map(params![like, limit], row_to_file)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The chain that produced this file, oldest first. This is what lets the
    /// gallery show "original scan → compressed → signed" rather than orphans.
    pub fn lineage(&self, hash: &str) -> Result<Vec<JobRecord>> {
        let mut chain = Vec::new();
        let mut current = hash.to_string();
        // Guard against a cycle, which shouldn't happen but would hang the UI.
        for _ in 0..64 {
            let job: Option<JobRecord> = self
                .db
                .query_row(
                    "SELECT * FROM jobs WHERE output_hash = ?1 ORDER BY created_at DESC LIMIT 1",
                    params![current],
                    row_to_job,
                )
                .optional()?;
            match job {
                Some(j) => {
                    current = j.input_hash.clone();
                    chain.push(j);
                }
                None => break,
            }
        }
        chain.reverse();
        Ok(chain)
    }

    /// Everything produced from this file, directly or indirectly.
    pub fn derivatives(&self, hash: &str) -> Result<Vec<JobRecord>> {
        let mut stmt =
            self.db.prepare("SELECT * FROM jobs WHERE input_hash = ?1 ORDER BY created_at DESC")?;
        let rows = stmt.query_map(params![hash], row_to_job)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Say when a stored copy really dates from.
    ///
    /// Two uses: a file brought in from a backup should carry its own date
    /// rather than the date it was imported, and the retention rule — which
    /// decides by age — can only be tested if a test can make something old.
    pub fn set_created_at(&self, hash: &str, when_ms: i64) -> Result<()> {
        self.db
            .execute("UPDATE files SET created_at = ?2 WHERE hash = ?1", params![hash, when_ms])?;
        Ok(())
    }

    pub fn set_pinned(&self, hash: &str, pinned: bool) -> Result<()> {
        self.db.execute(
            "UPDATE files SET pinned = ?2 WHERE hash = ?1",
            params![hash, pinned as i32],
        )?;
        Ok(())
    }

    /// Remove a file from the index and delete its bytes.
    ///
    /// "Forget" has to mean forget, so this takes the row, its jobs, its tags,
    /// its blob and its thumbnail's blob — a thumbnail left behind is a
    /// picture of something the person asked to be rid of.
    ///
    /// A blob is kept only while something still points at it: the store is
    /// content-addressed, so the same bytes stored twice are one file on disk
    /// and forgetting one copy must not take the other's.
    pub fn forget(&self, hash: &str) -> Result<()> {
        let record = self.get(hash)?;

        // Only the jobs this file was the *result* of. A job whose input
        // has been forgotten is still a true record of something that
        // happened, and the gallery is built on jobs: deleting one because
        // its input went took the finished file off the screen with it, even
        // though the file itself was kept. That is what the default retention
        // rule did - sweep the originals after ninety days, and every result
        // quietly vanished from history while sitting on disk.
        //
        // `entry_story` has always drawn this case as "a file no longer
        // kept", so a dangling input was expected here long before it could
        // actually happen.
        self.db.execute("DELETE FROM jobs WHERE output_hash = ?1", params![hash])?;
        self.db.execute("DELETE FROM files WHERE hash = ?1", params![hash])?;
        self.db.execute("DELETE FROM tags WHERE hash = ?1", params![hash])?;

        self.drop_blob_if_unused(hash)?;
        if let Some(thumb) = record.and_then(|r| r.thumb_hash) {
            self.drop_blob_if_unused(&thumb)?;
        }
        Ok(())
    }

    /// Delete the bytes behind a hash, unless a row still points at them.
    fn drop_blob_if_unused(&self, hash: &str) -> Result<()> {
        let referenced: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM files WHERE hash = ?1 OR thumb_hash = ?1",
            params![hash],
            |r| r.get(0),
        )?;
        if referenced > 0 {
            return Ok(());
        }
        let path = self.blob_path(hash);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn usage(&self) -> Result<Usage> {
        let (files, bytes): (i64, i64) =
            self.db.query_row("SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        let jobs: i64 = self.db.query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))?;
        Ok(Usage { files: files as u64, bytes: bytes as u64, jobs: jobs as u64 })
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        // Two levels of fan-out keeps directory listings manageable even with
        // tens of thousands of files.
        let (a, b) = (&hash[0..2], &hash[2..4]);
        self.root.join("blobs").join(a).join(b).join(hash)
    }
}

fn row_to_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileRecord> {
    Ok(FileRecord {
        hash: row.get("hash")?,
        size: row.get::<_, i64>("size")? as u64,
        name: row.get("name")?,
        kind: row.get("kind")?,
        width: row.get("width")?,
        height: row.get("height")?,
        pages: row.get("pages")?,
        source_path: row.get("source_path")?,
        thumb_hash: row.get("thumb_hash")?,
        created_at: row.get("created_at")?,
        pinned: row.get::<_, i64>("pinned")? != 0,
    })
}

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRecord> {
    Ok(JobRecord {
        id: row.get("id")?,
        tool: row.get("tool")?,
        recipe: row.get("recipe")?,
        input_hash: row.get("input_hash")?,
        output_hash: row.get("output_hash")?,
        summary: row.get("summary")?,
        duration_ms: row.get("duration_ms")?,
        created_at: row.get("created_at")?,
    })
}

fn now_millis() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        (dir, s)
    }

    fn meta(name: &str) -> FileMeta {
        FileMeta { name: name.into(), kind: "jpeg".into(), ..Default::default() }
    }

    #[test]
    fn stores_and_reads_back() {
        let (_d, s) = store();
        let rec = s.put(b"hello form", meta("scan.jpg")).unwrap();
        assert_eq!(s.read(&rec.hash).unwrap(), b"hello form");
        assert_eq!(s.get(&rec.hash).unwrap().unwrap().name, "scan.jpg");
    }

    #[test]
    fn forgetting_takes_the_row_the_job_and_the_bytes() {
        // "Forget" has to mean forget: a row that stays, or a blob left on
        // disk, is the difference between deleting something and hiding it.
        let (dir, s) = store();
        let file = s.put(b"a private scan", meta("passport.jpg")).unwrap();
        s.record_job(NewJob {
            tool: "compress".into(),
            recipe: "{}".into(),
            input_hash: file.hash.clone(),
            output_hash: file.hash.clone(),
            summary: "done".into(),
            duration_ms: 1,
        })
        .unwrap();
        let blob =
            dir.path().join("blobs").join(&file.hash[0..2]).join(&file.hash[2..4]).join(&file.hash);
        assert!(blob.exists());

        s.forget(&file.hash).unwrap();

        assert!(s.get(&file.hash).unwrap().is_none(), "the row is gone");
        assert_eq!(s.usage().unwrap().jobs, 0, "the job row is gone");
        assert!(!blob.exists(), "the bytes are gone from disk");
    }

    /// Forgetting the original must not take the result off the screen.
    ///
    /// The gallery lists files that are the output of some job, so a job is
    /// what makes a result visible. Deleting jobs by their input meant that
    /// clearing away an original - which is exactly what the retention rule
    /// is for - hid the finished file as well.
    #[test]
    fn forgetting_the_original_leaves_the_result_in_the_gallery() {
        let (_d, s) = store();
        let original = s.put(b"the original scan", meta("scan.jpg")).unwrap();
        let result = s.put(b"the compressed copy", meta("scan-kitbox.jpg")).unwrap();
        s.record_job(NewJob {
            tool: "compress".into(),
            recipe: "{}".into(),
            input_hash: original.hash.clone(),
            output_hash: result.hash.clone(),
            summary: "squeezed".into(),
            duration_ms: 1,
        })
        .unwrap();

        s.forget(&original.hash).unwrap();

        assert!(s.get(&original.hash).unwrap().is_none(), "the original is gone, as asked");
        assert_eq!(
            s.recent_outputs(10).unwrap().len(),
            1,
            "the result it produced is still what the gallery shows"
        );
        let chain = s.lineage(&result.hash).unwrap();
        assert_eq!(chain.len(), 1, "and the step that made it is still recorded");
    }

    #[test]
    fn forgetting_takes_the_thumbnail_with_it() {
        // A thumbnail left behind is a picture of the thing someone asked to
        // be rid of.
        let (dir, s) = store();
        let thumb = s.put(b"thumbnail bytes", meta("thumb.jpg")).unwrap();
        let file = s
            .put(
                b"the document itself",
                FileMeta {
                    name: "scan.jpg".into(),
                    kind: "jpeg".into(),
                    thumb_hash: Some(thumb.hash.clone()),
                    ..Default::default()
                },
            )
            .unwrap();

        // The thumbnail's own row is what a real one never has; drop it so the
        // only thing pointing at those bytes is the file.
        s.db.execute("DELETE FROM files WHERE hash = ?1", params![thumb.hash]).unwrap();

        s.forget(&file.hash).unwrap();

        let thumb_blob = dir
            .path()
            .join("blobs")
            .join(&thumb.hash[0..2])
            .join(&thumb.hash[2..4])
            .join(&thumb.hash);
        assert!(!thumb_blob.exists(), "the thumbnail's bytes should go too");
    }

    #[test]
    fn forgetting_one_copy_leaves_the_other_alone() {
        // Content addressing means two entries can share one file on disk.
        // Forgetting one must not reach into the other.
        let (dir, s) = store();
        let first = s.put(b"shared bytes", meta("one.jpg")).unwrap();
        let second = s.put(b"shared bytes", meta("two.jpg")).unwrap();
        assert_eq!(first.hash, second.hash, "the same bytes, stored once");

        s.forget(&first.hash).unwrap();

        // One hash, one row: forgetting it is forgetting both names, and the
        // blob goes with it. What must not happen is a row with no bytes.
        let blob = dir
            .path()
            .join("blobs")
            .join(&first.hash[0..2])
            .join(&first.hash[2..4])
            .join(&first.hash);
        assert_eq!(s.get(&first.hash).unwrap().is_none(), !blob.exists());
    }

    #[test]
    fn a_copy_no_job_mentions_is_findable_so_it_can_be_cleared() {
        // Inputs are kept so a chain stays whole. Once the jobs are gone the
        // copy is unreachable — not in the gallery, not in any lineage — and
        // would otherwise sit there forever.
        let (_d, s) = store();
        let input = s.put(b"what went in", meta("original.jpg")).unwrap();
        let output = s.put(b"what came out", meta("original-kitbox.jpg")).unwrap();
        s.record_job(NewJob {
            tool: "compress".into(),
            recipe: "{}".into(),
            input_hash: input.hash.clone(),
            output_hash: output.hash.clone(),
            summary: "done".into(),
            duration_ms: 1,
        })
        .unwrap();

        assert!(s.unreachable().unwrap().is_empty(), "both are reachable while the job stands");

        s.forget(&output.hash).unwrap();

        let loose = s.unreachable().unwrap();
        assert_eq!(loose.len(), 1, "the input is now reachable from nothing");
        assert_eq!(loose[0].hash, input.hash);
    }

    /// The case the test above does not reach.
    ///
    /// A thumbnail is a file row like any other and no job ever names one, so
    /// a sweep that asks only about jobs finds every thumbnail in the store
    /// and calls it loose: the storage panel opens offering to tidy away one
    /// thing per entry, none of which are junk. Found by running the history
    /// screen on Android, where the same store and the same sweep live.
    #[test]
    fn a_thumbnail_is_not_loose_while_its_picture_is_in_the_gallery() {
        let (_d, s) = store();

        let thumb = s.put(b"a small jpeg", meta("photo.jpg.thumb")).unwrap();
        let photo = s
            .put(
                b"the photograph",
                FileMeta { thumb_hash: Some(thumb.hash.clone()), ..meta("photo.jpg") },
            )
            .unwrap();
        s.record_job(NewJob {
            tool: "compress".into(),
            recipe: "{}".into(),
            input_hash: String::new(),
            output_hash: photo.hash.clone(),
            summary: "done".into(),
            duration_ms: 1,
        })
        .unwrap();

        assert!(
            s.unreachable().unwrap().is_empty(),
            "the thumbnail is what the gallery draws; nothing about it is loose"
        );

        // And once the picture itself is gone, the thumbnail goes with it
        // rather than being left behind - which is the other half of the
        // promise, and is `forget`'s job rather than this one's.
        s.forget(&photo.hash).unwrap();
        assert!(s.get(&thumb.hash).unwrap().is_some(), "the row is still there to be swept");
        assert_eq!(
            s.unreachable().unwrap().len(),
            1,
            "with nothing pointing at it, the thumbnail is loose now"
        );
    }

    #[test]
    fn everything_can_be_listed_for_the_storage_panel() {
        let (_d, s) = store();
        s.put(b"one", meta("a.jpg")).unwrap();
        s.put(b"two", meta("b.jpg")).unwrap();
        let all = s.all_files().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn identical_bytes_are_stored_once() {
        let (_d, s) = store();
        let a = s.put(b"same", meta("a.jpg")).unwrap();
        let b = s.put(b"same", meta("b.jpg")).unwrap();
        assert_eq!(a.hash, b.hash);
        assert_eq!(s.usage().unwrap().files, 1, "deduplication should keep one row");
    }

    #[test]
    fn lineage_walks_back_to_the_original() {
        let (_d, s) = store();
        let original = s.put(b"original scan", meta("scan.jpg")).unwrap();
        let compressed = s.put(b"compressed", meta("scan-87kb.jpg")).unwrap();
        let signed = s.put(b"signed", meta("scan-signed.jpg")).unwrap();

        s.record_job(NewJob {
            tool: "compress".into(),
            recipe: "{}".into(),
            input_hash: original.hash.clone(),
            output_hash: compressed.hash.clone(),
            summary: "1.5 MB → 87 KB".into(),
            duration_ms: 1200,
        })
        .unwrap();
        s.record_job(NewJob {
            tool: "sign".into(),
            recipe: "{}".into(),
            input_hash: compressed.hash.clone(),
            output_hash: signed.hash.clone(),
            summary: "signed".into(),
            duration_ms: 40,
        })
        .unwrap();

        let chain = s.lineage(&signed.hash).unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].input_hash, original.hash, "chain should start at the original");
        assert_eq!(chain[1].tool, "sign");

        let recent = s.recent_outputs(10).unwrap();
        assert_eq!(recent.len(), 2, "only job outputs appear in recents");
    }

    #[test]
    fn forgetting_removes_the_blob() {
        let (_d, s) = store();
        let rec = s.put(b"private document", meta("passport.jpg")).unwrap();
        s.forget(&rec.hash).unwrap();
        assert!(s.get(&rec.hash).unwrap().is_none());
        assert!(matches!(s.read(&rec.hash), Err(Error::NotFound(_))));
    }

    #[test]
    fn search_matches_on_name() {
        let (_d, s) = store();
        s.put(b"a", meta("upsc-photo.jpg")).unwrap();
        s.put(b"b", meta("holiday.jpg")).unwrap();
        let hits = s.search("upsc", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "upsc-photo.jpg");
    }
}
