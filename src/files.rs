use std::fs::{self, File};
use std::io::Write;
use std::mem::DropGuard;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::InternalResult;

pub(crate) fn replace(path: &Path, value: &[u8]) -> InternalResult<()> {
    static NEXT_TEMPORARY: AtomicUsize = AtomicUsize::new(0);
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
        let mut file = File::create_new(&temporary)?;
        let cleanup = DropGuard::new(&temporary, |path| {
            let _ = fs::remove_file(path);
        });
        let written = file.write_all(value);
        drop(file);
        written?;
        fs::rename(&temporary, path)?;
        DropGuard::dismiss(cleanup);
        Ok(())
    })();
    result.map_err(|error| format!("could not write {}: {error}", path.display()).into())
}
