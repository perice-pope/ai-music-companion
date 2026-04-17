//! # Retention — age + size-bounded cleanup for WAV recordings
//!
//! A [`RetentionManager`] sweeps a directory of `.wav` files and deletes the
//! ones that no longer fit the [`RetentionPolicy`]. Two bounds are applied
//! in order:
//!
//! 1. **Age sweep** — files older than `max_age_days` are deleted (unless
//!    they match the best-take preservation rule).
//! 2. **Size sweep** — if `max_total_bytes` is set and the remaining files
//!    still exceed it, the oldest are deleted until the directory fits.
//!
//! Best-take preservation uses a filename convention: any file whose stem
//! ends in `_best` (e.g. `foo_best.wav`) is kept regardless of age — as
//! long as `keep_marked_best_takes` is `true`. Size-sweeping still considers
//! best-takes so a pathological all-best-takes directory is not left
//! unbounded; callers can set `max_total_bytes` to `None` to truly preserve.
//!
//! Non-`.wav` files in the directory are ignored entirely.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::recorder::RecorderError;

/// Policy applied by [`RetentionManager::sweep`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// Delete files older than this many days. `None` disables the age sweep.
    pub max_age_days: Option<u32>,
    /// Cap on total bytes across all remaining WAV files after the age sweep.
    /// Oldest files are deleted first. `None` disables the size sweep.
    pub max_total_bytes: Option<u64>,
    /// When true, files whose stem ends in `_best` are preserved regardless
    /// of age. They are still candidates for the size sweep.
    pub keep_marked_best_takes: bool,
}

/// Summary of a single [`RetentionManager::sweep`] pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SweepReport {
    pub files_deleted: usize,
    pub bytes_reclaimed: u64,
    pub files_kept: usize,
}

/// Sweeper applying a [`RetentionPolicy`] to a directory.
pub struct RetentionManager {
    policy: RetentionPolicy,
}

impl RetentionManager {
    /// Build a new manager with the given policy.
    pub fn new(policy: RetentionPolicy) -> Self {
        Self { policy }
    }

    /// Return a reference to the configured policy.
    pub fn policy(&self) -> &RetentionPolicy {
        &self.policy
    }

    /// Scan `dir` and apply the retention policy, deleting files as needed.
    pub fn sweep(&self, dir: &Path) -> Result<SweepReport, RecorderError> {
        if !dir.exists() {
            return Err(RecorderError::OutputDirMissing(dir.to_path_buf()));
        }

        let mut entries = collect_wav_entries(dir)?;

        let mut files_deleted = 0usize;
        let mut bytes_reclaimed = 0u64;

        // 1. Age sweep.
        if let Some(max_age_days) = self.policy.max_age_days {
            let cutoff = SystemTime::now()
                .checked_sub(Duration::from_secs(u64::from(max_age_days) * 24 * 60 * 60))
                .unwrap_or(SystemTime::UNIX_EPOCH);

            entries.retain(|entry| {
                let is_best = self.policy.keep_marked_best_takes && entry.is_best;
                if !is_best && entry.modified < cutoff {
                    match fs::remove_file(&entry.path) {
                        Ok(()) => {
                            files_deleted += 1;
                            bytes_reclaimed += entry.size;
                            false
                        }
                        Err(_) => true, // keep entry if deletion failed
                    }
                } else {
                    true
                }
            });
        }

        // 2. Size sweep — oldest first.
        if let Some(max_total_bytes) = self.policy.max_total_bytes {
            let mut total: u64 = entries.iter().map(|e| e.size).sum();
            if total > max_total_bytes {
                entries.sort_by_key(|e| e.modified);
                let mut i = 0;
                while total > max_total_bytes && i < entries.len() {
                    let entry = &entries[i];
                    match fs::remove_file(&entry.path) {
                        Ok(()) => {
                            files_deleted += 1;
                            bytes_reclaimed += entry.size;
                            total = total.saturating_sub(entry.size);
                            entries.remove(i);
                            // do not advance i, next element shifts into place
                        }
                        Err(_) => i += 1,
                    }
                }
            }
        }

        Ok(SweepReport {
            files_deleted,
            bytes_reclaimed,
            files_kept: entries.len(),
        })
    }
}

struct WavEntry {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
    is_best: bool,
}

fn collect_wav_entries(dir: &Path) -> Result<Vec<WavEntry>, RecorderError> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("wav") {
            continue;
        }
        let metadata = entry.metadata()?;
        let size = metadata.len();
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let is_best = stem.ends_with("_best");
        out.push(WavEntry {
            path,
            size,
            modified,
            is_best,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_file(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut f = File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        path
    }

    fn set_mtime_days_ago(path: &Path, days: u64) {
        let target = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
        let ft = filetime::FileTime::from_system_time(target);
        filetime::set_file_mtime(path, ft).unwrap();
    }

    #[test]
    fn sweep_deletes_files_older_than_max_age() {
        let tmp = TempDir::new().unwrap();
        let old = write_file(tmp.path(), "old.wav", &[0u8; 128]);
        let fresh = write_file(tmp.path(), "fresh.wav", &[0u8; 128]);
        set_mtime_days_ago(&old, 5);

        let mgr = RetentionManager::new(RetentionPolicy {
            max_age_days: Some(1),
            max_total_bytes: None,
            keep_marked_best_takes: true,
        });
        let report = mgr.sweep(tmp.path()).unwrap();

        assert!(!old.exists(), "old file should be deleted");
        assert!(fresh.exists(), "fresh file should be kept");
        assert_eq!(report.files_deleted, 1);
        assert_eq!(report.bytes_reclaimed, 128);
        assert_eq!(report.files_kept, 1);
    }

    #[test]
    fn sweep_preserves_best_take_files_when_flag_set() {
        let tmp = TempDir::new().unwrap();
        let best = write_file(tmp.path(), "session_best.wav", &[0u8; 128]);
        set_mtime_days_ago(&best, 30);

        let mgr = RetentionManager::new(RetentionPolicy {
            max_age_days: Some(1),
            max_total_bytes: None,
            keep_marked_best_takes: true,
        });
        let report = mgr.sweep(tmp.path()).unwrap();
        assert!(best.exists(), "best-take file must be preserved");
        assert_eq!(report.files_deleted, 0);
        assert_eq!(report.files_kept, 1);
    }

    #[test]
    fn sweep_deletes_best_take_when_flag_off() {
        let tmp = TempDir::new().unwrap();
        let best = write_file(tmp.path(), "session_best.wav", &[0u8; 128]);
        set_mtime_days_ago(&best, 30);

        let mgr = RetentionManager::new(RetentionPolicy {
            max_age_days: Some(1),
            max_total_bytes: None,
            keep_marked_best_takes: false,
        });
        let report = mgr.sweep(tmp.path()).unwrap();
        assert!(!best.exists());
        assert_eq!(report.files_deleted, 1);
    }

    #[test]
    fn sweep_respects_max_total_bytes() {
        let tmp = TempDir::new().unwrap();
        let a = write_file(tmp.path(), "a.wav", &[0u8; 1024]);
        let b = write_file(tmp.path(), "b.wav", &[0u8; 1024]);
        let c = write_file(tmp.path(), "c.wav", &[0u8; 1024]);
        // Make a the oldest, c the newest.
        set_mtime_days_ago(&a, 3);
        set_mtime_days_ago(&b, 2);
        set_mtime_days_ago(&c, 1);

        let mgr = RetentionManager::new(RetentionPolicy {
            max_age_days: None,
            max_total_bytes: Some(2048),
            keep_marked_best_takes: false,
        });
        let report = mgr.sweep(tmp.path()).unwrap();

        assert!(!a.exists(), "oldest should be deleted first");
        assert!(b.exists());
        assert!(c.exists());
        assert_eq!(report.files_deleted, 1);
        assert_eq!(report.bytes_reclaimed, 1024);
        assert_eq!(report.files_kept, 2);
    }

    #[test]
    fn sweep_reports_counts_and_bytes() {
        let tmp = TempDir::new().unwrap();
        let a = write_file(tmp.path(), "a.wav", &[0u8; 256]);
        let b = write_file(tmp.path(), "b.wav", &[0u8; 128]);
        set_mtime_days_ago(&a, 10);
        set_mtime_days_ago(&b, 10);

        let mgr = RetentionManager::new(RetentionPolicy {
            max_age_days: Some(1),
            max_total_bytes: None,
            keep_marked_best_takes: false,
        });
        let report = mgr.sweep(tmp.path()).unwrap();
        assert_eq!(report.files_deleted, 2);
        assert_eq!(report.bytes_reclaimed, 384);
        assert_eq!(report.files_kept, 0);
    }

    #[test]
    fn sweep_handles_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let mgr = RetentionManager::new(RetentionPolicy {
            max_age_days: Some(1),
            max_total_bytes: Some(1024),
            keep_marked_best_takes: true,
        });
        let report = mgr.sweep(tmp.path()).unwrap();
        assert_eq!(
            report,
            SweepReport {
                files_deleted: 0,
                bytes_reclaimed: 0,
                files_kept: 0,
            }
        );
    }

    #[test]
    fn sweep_ignores_non_wav_files() {
        let tmp = TempDir::new().unwrap();
        let wav = write_file(tmp.path(), "session.wav", &[0u8; 64]);
        let txt = write_file(tmp.path(), "notes.txt", b"hello");
        set_mtime_days_ago(&wav, 30);
        set_mtime_days_ago(&txt, 30);

        let mgr = RetentionManager::new(RetentionPolicy {
            max_age_days: Some(1),
            max_total_bytes: None,
            keep_marked_best_takes: false,
        });
        let report = mgr.sweep(tmp.path()).unwrap();
        assert!(!wav.exists());
        assert!(txt.exists(), "non-wav files must not be touched");
        assert_eq!(report.files_deleted, 1);
    }

    #[test]
    fn sweep_missing_dir_errors() {
        let tmp = TempDir::new().unwrap();
        let ghost = tmp.path().join("not-a-dir");
        let mgr = RetentionManager::new(RetentionPolicy {
            max_age_days: Some(1),
            max_total_bytes: None,
            keep_marked_best_takes: false,
        });
        let err = mgr.sweep(&ghost).unwrap_err();
        match err {
            RecorderError::OutputDirMissing(p) => assert_eq!(p, ghost),
            other => panic!("expected OutputDirMissing, got {other:?}"),
        }
    }

    #[test]
    fn size_sweep_can_delete_best_take_if_needed_for_cap() {
        // Document the edge case: size sweep is not age-based and still
        // reclaims best-takes to stay under max_total_bytes. Callers who
        // want absolute preservation of best-takes should leave
        // max_total_bytes as None.
        let tmp = TempDir::new().unwrap();
        let best = write_file(tmp.path(), "a_best.wav", &[0u8; 2048]);
        set_mtime_days_ago(&best, 2);
        let other = write_file(tmp.path(), "b.wav", &[0u8; 2048]);
        set_mtime_days_ago(&other, 1);

        let mgr = RetentionManager::new(RetentionPolicy {
            max_age_days: None,
            max_total_bytes: Some(2048),
            keep_marked_best_takes: true,
        });
        let report = mgr.sweep(tmp.path()).unwrap();
        // Oldest removed first; best-take is the oldest here.
        assert!(!best.exists());
        assert!(other.exists());
        assert_eq!(report.files_deleted, 1);
    }
}
