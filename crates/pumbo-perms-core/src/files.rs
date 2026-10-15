//! Files in the plugin's data folder, the same on every platform: exports
//! (`exports/<name>.json`) and the import of a whole export file.

use std::io::Write;
use std::path::Path;

use pumbo_common::text::Args;

use crate::command::{self, Outcome, Who};
use crate::engine::Engine;
use crate::export;
use crate::ui::{self, Text, value};

/// Writes the whole database to `<dir>/exports/<name>.json` and returns that
/// path as `exports/<name>.json`. Refused while the database is broken (memory
/// then holds nothing worth saving) and when the file exists. The data goes to
/// `<name>.json.tmp` first, so a crash never leaves half a file.
pub fn export_to(e: &Engine, dir: &Path, name: &str, now: u64) -> Result<String, Text> {
    if e.broken.is_some() {
        return Err(ui::error(&e.lang, "database-broken", &Args::new()));
    }
    let shown = format!("exports/{name}.json");
    let folder = dir.join("exports");
    let path = folder.join(format!("{name}.json"));
    if path.exists() {
        return Err(ui::error(&e.lang, "export-exists", &Args::new().arg(value(&shown))));
    }
    let tmp = folder.join(format!("{name}.json.tmp"));
    let json = export::dump(&e.perms, now);
    let written = std::fs::create_dir_all(&folder)
        .and_then(|()| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.sync_all()
        })
        .and_then(|()| std::fs::rename(&tmp, &path));
    match written {
        Ok(()) => Ok(shown),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(ui::error(&e.lang, "export-failed", &Args::new().arg(value(err))))
        }
    }
}

/// `/pp import <name>`: replaces everything with `<dir>/exports/<name>.json`
/// after saving the current data as `before-import-<time with ms>`.
pub fn import_file(e: &mut Engine, dir: &Path, who: &Who, name: &str, now: u64) -> Outcome {
    let Ok(json) = std::fs::read_to_string(dir.join("exports").join(format!("{name}.json"))) else {
        let reply = ui::error(&e.lang, "import-missing", &Args::new().arg(value(format!("exports/{name}.json"))));
        return Outcome { reply, ..Outcome::default() };
    };
    let backup = format!("before-import-{}-{:03}", export::default_file_name(now), now % 1000);
    match export_to(e, dir, &backup, now) {
        Ok(path) => command::import(e, who, name, &json, &path, now),
        Err(reply) => Outcome { reply, ..Outcome::default() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CoreCfg;
    use pumbo_common::lang::{COMMON, Lang};

    fn engine() -> Engine {
        Engine::in_memory(CoreCfg::default(), Lang::load(&[COMMON, crate::LANG], "en", None).0)
    }

    fn dir(test: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("pumbo-perms-files-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn export_refuses_existing_name() {
        let (e, d) = (engine(), dir("exists"));
        assert_eq!(export_to(&e, &d, "nightly", 0).unwrap(), "exports/nightly.json");
        std::fs::write(d.join("exports/nightly.json"), "precious").unwrap();
        let out = export_to(&e, &d, "nightly", 0).unwrap_err().plain();
        assert!(out.contains("exports/nightly.json exists, choose another name"), "{out}");
        assert_eq!(std::fs::read_to_string(d.join("exports/nightly.json")).unwrap(), "precious");
    }

    #[test]
    fn export_writes_through_tmp() {
        let (e, d) = (engine(), dir("tmp"));
        // The temporary file cannot be written: the export fails and leaves no file behind.
        std::fs::create_dir_all(d.join("exports/nightly.json.tmp")).unwrap();
        assert!(export_to(&e, &d, "nightly", 0).unwrap_err().plain().contains("Export failed"));
        assert!(!d.join("exports/nightly.json").exists());
        std::fs::remove_dir(d.join("exports/nightly.json.tmp")).unwrap();
        export_to(&e, &d, "nightly", 0).unwrap();
        assert!(export::parse(&std::fs::read_to_string(d.join("exports/nightly.json")).unwrap()).is_ok());
        assert!(!d.join("exports/nightly.json.tmp").exists());
    }

    #[test]
    fn import_keeps_a_backup_with_milliseconds() {
        let (mut e, d) = (engine(), dir("import"));
        export_to(&e, &d, "start", 1_500).unwrap();
        let out = import_file(&mut e, &d, &Who::console(), "start", 1_500);
        assert!(out.reply.plain().contains("before-import-pumboperms-1970-01-01-000001-500"), "{}", out.reply.plain());
        // A second import in the same second gets its own backup.
        let out = import_file(&mut e, &d, &Who::console(), "start", 1_501);
        assert!(out.changed, "{}", out.reply.plain());
    }
}
