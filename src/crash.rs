//! Game crashes: when an app crashes, the PS5 writes /user/devlog/system/sce_coredumps.0/<TID>_<n>/
//! (the dump `prosperocore-<n>-0x<pid>-<process>.prosperodmp` inside) and deletes it seconds later.
//! The launch watcher looks there when a game it mounted exits and once more when that session's
//! cleanup ends, matching the dump by pid (launch.rs on_exit). The last crash of each title is kept
//! in FILE, so it is still there after a payload restart or a reboot. See DESIGN.md "Crash report".
use std::collections::HashMap;
use std::path::Path;

pub const DIR: &str = "/user/devlog/system/sce_coredumps.0";
/// Not in the game's cache: for a pkg the cache is /app0, so the game would see it.
pub const FILE: &str = "/data/krakenmount/crashes.json";

#[derive(Clone, Debug, PartialEq)]
pub struct Crash {
    /// Unix seconds.
    pub at: u64,
    /// From the dump's file name ("eboot.bin").
    pub process: String,
    /// The console's crash folder name ("PPSA22327_1795166239").
    pub folder: String,
    /// The exit status from NOTE_EXIT ("killed by signal 11").
    pub status: String,
}

/// "PPSA22327_1795166239" -> "PPSA22327".
pub fn folder_tid(name: &str) -> Option<&str> {
    let (tid, n) = name.split_once('_')?;
    let digits = !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    (digits && crate::scan::valid_tid(tid)).then_some(tid)
}

/// "prosperocore-1795166239-0x000000e9-eboot.bin.prosperodmp" -> (0xe9, "eboot.bin").
pub fn dump_pid_process(file: &str) -> Option<(libc::pid_t, &str)> {
    let rest = file.strip_prefix("prosperocore-")?.strip_suffix(".prosperodmp")?;
    let mut parts = rest.splitn(3, '-');
    let (_, pid, process) = (parts.next()?, parts.next()?, parts.next()?);
    let pid = libc::pid_t::from_str_radix(pid.strip_prefix("0x")?, 16).ok()?;
    (!process.is_empty()).then_some((pid, process))
}

/// The NOTE_EXIT data, a wait(2) status: "exited with 0", "killed by signal 11".
pub fn exit_status(status: i32) -> String {
    match status & 0x7f {
        0 => format!("exited with {}", (status >> 8) & 0xff),
        0x7f => format!("status {status:#x}"),
        sig => format!("killed by signal {sig}"),
    }
}

/// The dump of `pid` among the `<tid>_*` folders in `dir`: (folder, process).
pub fn find_in(dir: &Path, tid: &str, pid: libc::pid_t) -> Option<(String, String)> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let folder = e.file_name().to_string_lossy().into_owned();
        if folder_tid(&folder) != Some(tid) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(e.path()) else { continue };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if let Some((p, process)) = dump_pid_process(&name) {
                if p == pid {
                    return Some((folder, process.to_string()));
                }
            }
        }
    }
    None
}

pub fn load_from(path: &Path) -> HashMap<String, Crash> {
    let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&text) else { return HashMap::new() };
    let s = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    map.into_iter()
        .map(|(tid, v)| {
            let c = Crash { at: v["at"].as_u64().unwrap_or(0), process: s(&v, "process"), folder: s(&v, "folder"), status: s(&v, "status") };
            (tid, c)
        })
        .collect()
}

/// Temp file + rename: a crash while writing never leaves a half file.
pub fn save_to(path: &Path, crashes: &HashMap<String, Crash>) -> std::io::Result<()> {
    let map: serde_json::Map<String, serde_json::Value> = crashes
        .iter()
        .map(|(tid, c)| {
            let v = serde_json::json!({"at": c.at, "process": c.process, "folder": c.folder, "status": c.status});
            (tid.clone(), v)
        })
        .collect();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::Value::Object(map).to_string())?;
    std::fs::rename(&tmp, path)
}

/// The last crash of each title, from FILE.
pub fn load() -> HashMap<String, Crash> {
    load_from(Path::new(FILE))
}

/// Looks for the dump of `pid`; when there is one, records it as `tid`'s last crash. `when` is for the log.
pub fn check(tid: &str, pid: libc::pid_t, status: &str, when: &str) -> bool {
    let Some((folder, process)) = find_in(Path::new(DIR), tid, pid) else { return false };
    log!("{tid}: crashed ({process}, {status}), dump found {when}; the console's crash dump: {DIR}/{folder}");
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut all = load();
    all.insert(tid.to_string(), Crash { at, process, folder, status: status.to_string() });
    if let Err(e) = save_to(Path::new(FILE), &all) {
        log!("{tid}: could not save {FILE}: {e}");
    }
    true
}
