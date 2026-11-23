//! exFAT game images (`.exfat`, DESIGN.md "Images"): a port of SMP sm_image.c / sm_mount_device.c,
//! LVD backend only. The image is attached through /dev/lvdctl as /dev/lvdN and mounted read-only
//! (exfatfs) on /mnt/krakenmount/<stem>_<fnv32 of the image path>; the game inside is then an
//! ordinary folder source. /data/krakenmount/images/<mount name> records `<unit>\n<image path>`
//! so a later copy of the payload (no reboot) finds, reuses or releases its mounts.
use std::collections::HashSet;
use std::ffi::CString;
use std::fs;
use std::io::{self, Error, ErrorKind};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const MOUNT_BASE: &str = "/mnt/krakenmount";
const STATE_DIR: &str = "/data/krakenmount/images";

const LVD_CTRL: &str = "/dev/lvdctl";
const LVD_IOC_ATTACH_V0: libc::c_ulong = 0xC028_6D00;
const LVD_IOC_DETACH: libc::c_ulong = 0xC028_6D01;
const LVD_ENTRY_TYPE_FILE: u16 = 1;
const LVD_ENTRY_FLAG_NO_BITMAP: u16 = 0x1;
/// SMP LVD_ATTACH_FLAGS_SINGLE_RO: BASE | SINGLE_SAVE_PROFILE | RO_VARIANT.
const LVD_FLAGS_SINGLE_RO: u16 = 0x0004 | 0x0010 | 0x0008;
const LVD_IMAGE_TYPE_SINGLE: u16 = 0;
const LVD_IMAGE_TYPE_SAVE_DATA: u16 = 5;
/// SMP LVD_SECTOR_SIZE_EXFAT and LVD_SECONDARY_UNIT_IMAGE_IO.
const SECTOR: u32 = 512;
const SECONDARY_UNIT: u32 = 0x10000;

#[repr(C)]
struct LvdLayer {
    source_type: u16,
    flags: u16,
    reserved0: u32,
    path: *const libc::c_char,
    offset: u64,
    size: u64,
    bitmap_path: *const libc::c_char,
    bitmap_offset: u64,
    bitmap_size: u64,
}

#[repr(C)]
struct LvdAttach {
    io_version: u32,
    device_id: i32,
    sector_size: u32,
    secondary_unit: u32,
    flags: u16,
    image_type: u16,
    layer_count: u32,
    device_size: u64,
    layers: *mut LvdLayer,
}

#[repr(C)]
struct LvdDetach {
    reserved0: u32,
    device_id: i32,
    reserved: [u8; 0x20],
}

const _: () = assert!(std::mem::size_of::<LvdAttach>() == 0x28);
const _: () = assert!(std::mem::size_of::<LvdDetach>() == 0x28);

/// A `.exfat` file (any case).
pub fn is_image(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exfat"))
}

fn fnv1a32(s: &[u8]) -> u32 {
    s.iter().fold(0x811c_9dc5u32, |h, &b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// /mnt/krakenmount/<stem>_<hash>. ponytail: the stem is cut to 32 ASCII alnum/-/_ characters so
/// the path fits statfs f_mntonname (88 bytes); the hash of the full path keeps it unique.
pub fn mount_point(image: &Path) -> PathBuf {
    let stem: String = image.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        .chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(32).collect();
    let hash = fnv1a32(image.as_os_str().as_encoded_bytes());
    Path::new(MOUNT_BASE).join(format!("{stem}_{hash:08x}"))
}

/// The image mount point holding `path` (a game folder in an image), if any.
pub fn mount_of(path: &Path) -> Option<PathBuf> {
    let name = path.strip_prefix(MOUNT_BASE).ok()?.components().next()?;
    Some(Path::new(MOUNT_BASE).join(name))
}

fn state_file(mp: &Path) -> PathBuf {
    Path::new(STATE_DIR).join(mp.file_name().unwrap_or_default())
}

/// (unit, image path) recorded for the mount point `mp`.
fn recorded(mp: &Path) -> Option<(i32, PathBuf)> {
    let text = fs::read_to_string(state_file(mp)).ok()?;
    let (unit, image) = text.split_once('\n')?;
    Some((unit.trim().parse().ok()?, PathBuf::from(image)))
}

/// statfs of `mp` when it is a mount point: (fstype, from). Err for a dead mount (USB gone).
fn mounted_at(mp: &Path) -> io::Result<Option<(String, String)>> {
    let c = CString::new(mp.as_os_str().as_encoded_bytes()).map_err(|_| Error::from(ErrorKind::InvalidInput))?;
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        let e = Error::last_os_error();
        return if e.kind() == ErrorKind::NotFound { Ok(None) } else { Err(e) };
    }
    let s = |b: &[libc::c_char]| unsafe { std::ffi::CStr::from_ptr(b.as_ptr()) }.to_string_lossy().into_owned();
    if s(&st.f_mntonname) != mp.to_string_lossy() {
        return Ok(None);
    }
    Ok(Some((s(&st.f_fstypename), s(&st.f_mntfromname))))
}

/// Images whose game folder could not be found, by (path, size, mtime): not mounted again until
/// the file changes, else every scan would attach and release it.
static BAD: OnceLock<Mutex<HashSet<(PathBuf, u64, i64)>>> = OnceLock::new();

/// The game folder in a mounted image: its root when it holds eboot.bin and sce_sys/param.json,
/// else the first such folder directly inside it (sorted).
pub fn game_dir(mp: &Path) -> Option<PathBuf> {
    let is_game = |p: &Path| p.join("eboot.bin").is_file() && p.join("sce_sys/param.json").is_file();
    if is_game(mp) {
        return Some(mp.to_path_buf());
    }
    let mut dirs: Vec<PathBuf> = fs::read_dir(mp).ok()?.flatten().map(|e| e.path())
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) && p.is_dir())
        .collect();
    dirs.sort();
    dirs.into_iter().find(|p| is_game(p))
}

/// Mounts `image` when it is not mounted yet and returns its game folder. An existing mount of
/// this image (this run or an earlier copy of the payload) is reused; a dead one (its drive was
/// unplugged) is released first.
pub fn ensure(image: &Path) -> io::Result<PathBuf> {
    let meta = fs::metadata(image)?;
    let key = (image.to_path_buf(), meta.len(), meta.mtime());
    if BAD.get().is_some_and(|b| b.lock().unwrap_or_else(|e| e.into_inner()).contains(&key)) {
        return Err(Error::new(ErrorKind::InvalidData, "no game folder (eboot.bin + sce_sys/param.json) in the image"));
    }
    let mp = mount_point(image);
    match mounted_at(&mp) {
        Ok(Some((fstype, from))) if fstype == "exfatfs" && from.starts_with("/dev/lvd")
            && recorded(&mp).is_some_and(|(_, i)| i == image) => {}
        Ok(None) => mount(image, meta.len(), &mp)?,
        Ok(Some((fstype, from))) => {
            log!("{}: {} is a {fstype} of {from}, not ours; releasing it", image.display(), mp.display());
            release(&mp)?;
            mount(image, meta.len(), &mp)?;
        }
        Err(e) => {
            log!("{}: {}: {e}; releasing the dead mount", image.display(), mp.display());
            release(&mp)?;
            mount(image, meta.len(), &mp)?;
        }
    }
    match game_dir(&mp) {
        Some(dir) => Ok(dir),
        None => {
            if let Err(e) = release(&mp) {
                log!("{}: release {}: {e}", image.display(), mp.display());
            }
            BAD.get_or_init(|| Mutex::new(HashSet::new())).lock().unwrap_or_else(|e| e.into_inner()).insert(key);
            Err(Error::new(ErrorKind::InvalidData, "no game folder (eboot.bin + sce_sys/param.json) in the image"))
        }
    }
}

/// SMP get_image_mount_profile: an image on /data or /user is "optimized" (LVD image type
/// SAVE_DATA), anywhere else (USB, M.2) "legacy" (SINGLE).
fn image_type(image: &Path) -> u16 {
    if image.starts_with("/data") || image.starts_with("/user") {
        LVD_IMAGE_TYPE_SAVE_DATA
    } else {
        LVD_IMAGE_TYPE_SINGLE
    }
}

fn attach(image: &Path, size: u64) -> io::Result<i32> {
    let path = CString::new(image.as_os_str().as_encoded_bytes()).map_err(|_| Error::from(ErrorKind::InvalidInput))?;
    let ctl = CString::new(LVD_CTRL).unwrap();
    let fd = unsafe { libc::open(ctl.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        let e = Error::last_os_error();
        return Err(Error::new(e.kind(), format!("open {LVD_CTRL}: {e}")));
    }
    let mut layer = LvdLayer {
        source_type: LVD_ENTRY_TYPE_FILE, flags: LVD_ENTRY_FLAG_NO_BITMAP, reserved0: 0,
        path: path.as_ptr(), offset: 0, size, bitmap_path: std::ptr::null(), bitmap_offset: 0, bitmap_size: 0,
    };
    let mut req = LvdAttach {
        io_version: 0, device_id: -1, sector_size: SECTOR, secondary_unit: SECONDARY_UNIT,
        flags: LVD_FLAGS_SINGLE_RO, image_type: image_type(image), layer_count: 1, device_size: size,
        layers: &mut layer,
    };
    let rc = unsafe { libc::ioctl(fd, LVD_IOC_ATTACH_V0, &mut req as *mut LvdAttach) };
    let e = Error::last_os_error();
    unsafe { libc::close(fd) };
    if rc != 0 {
        return Err(Error::new(e.kind(), format!("lvd attach: {e}")));
    }
    if req.device_id < 0 {
        return Err(Error::other(format!("lvd attach returned unit {}", req.device_id)));
    }
    Ok(req.device_id)
}

fn detach(unit: i32) -> io::Result<()> {
    let ctl = CString::new(LVD_CTRL).unwrap();
    let fd = unsafe { libc::open(ctl.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return Err(Error::last_os_error());
    }
    let mut req = LvdDetach { reserved0: 0, device_id: unit, reserved: [0; 0x20] };
    let rc = unsafe { libc::ioctl(fd, LVD_IOC_DETACH, &mut req as *mut LvdDetach) };
    let e = Error::last_os_error();
    unsafe { libc::close(fd) };
    if rc != 0 { Err(e) } else { Ok(()) }
}

fn mount(image: &Path, size: u64, mp: &Path) -> io::Result<()> {
    fs::create_dir_all(mp)?;
    fs::create_dir_all(STATE_DIR)?;
    let unit = attach(image, size)?;
    let dev = format!("/dev/lvd{unit}");
    // SMP LVD_NODE_WAIT_RETRIES x LVD_NODE_WAIT_US.
    let mut node = false;
    for _ in 0..100 {
        if Path::new(&dev).exists() {
            node = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let fail = |e: Error| {
        if let Err(d) = detach(unit) {
            log!("{}: detach {dev}: {d}", image.display());
        }
        e
    };
    if !node {
        return Err(fail(Error::new(ErrorKind::TimedOut, format!("{dev} did not appear"))));
    }
    let fspath = mp.to_string_lossy();
    // SMP perform_image_nmount iov_exfat (without errmsg/force), read-only.
    let opts = [
        ("from", Some(dev.as_str())), ("fspath", Some(&*fspath)), ("fstype", Some("exfatfs")),
        ("budgetid", Some("game")), ("large", Some("yes")), ("timezone", Some("static")),
        ("async", None), ("noatime", None), ("ignoreacl", None), ("automounted", None),
    ];
    if let Err(e) = crate::sys::nmount_opt(&opts, libc::MNT_RDONLY as i32) {
        return Err(fail(Error::new(e.kind(), format!("nmount exfatfs {dev} -> {fspath}: {e}"))));
    }
    match mounted_at(mp) {
        Ok(Some((t, from))) if t == "exfatfs" && from == dev => {}
        other => {
            let _ = crate::sys::unmount(&fspath, 0);
            return Err(fail(Error::other(format!("{fspath}: not the expected exfatfs mount of {dev}: {other:?}"))));
        }
    }
    fs::write(state_file(mp), format!("{unit}\n{}", image.display()))?;
    log!("{}: mounted {dev} on {fspath} (exfatfs, read-only)", image.display());
    Ok(())
}

/// Unmounts `mp` (never MNT_FORCE: EBUSY = a title or a running game still uses it), detaches its
/// LVD unit and forgets it.
pub fn release(mp: &Path) -> io::Result<()> {
    let unit = match mounted_at(mp) {
        Ok(Some((_, from))) => from.strip_prefix("/dev/lvd").and_then(|u| u.parse().ok()),
        _ => None,
    }.or_else(|| recorded(mp).map(|(u, _)| u));
    let fspath = mp.to_string_lossy();
    match crate::sys::unmount(&fspath, 0) {
        Ok(()) => {}
        // EINVAL: nothing mounted there; ENOENT: no such dir.
        Err(e) if matches!(e.raw_os_error(), Some(libc::EINVAL | libc::ENOENT)) => {}
        Err(e) => return Err(Error::new(e.kind(), format!("unmount {fspath}: {e}"))),
    }
    if let Some(unit) = unit {
        // ENXIO/ENOENT/EINVAL: the unit is already gone. ponytail: the unit number is not checked
        // against the node identity as SMP does; a unit reused by the system in between is detached.
        if let Err(e) = detach(unit) {
            if !matches!(e.raw_os_error(), Some(libc::ENXIO | libc::ENOENT | libc::EINVAL)) {
                log!("{fspath}: detach /dev/lvd{unit}: {e}");
            }
        }
    }
    let _ = fs::remove_file(state_file(mp));
    let _ = fs::remove_dir(mp);
    log!("{fspath}: released");
    Ok(())
}

/// Our image mounts: (mount point, image path) from the state dir.
pub fn mounts() -> Vec<(PathBuf, PathBuf)> {
    let Ok(rd) = fs::read_dir(STATE_DIR) else { return Vec::new() };
    rd.flatten().filter_map(|e| {
        let mp = Path::new(MOUNT_BASE).join(e.file_name());
        recorded(&mp).map(|(_, image)| (mp, image))
    }).collect()
}

/// Unmounts the titles (/system_ex/app/<TID>) on top of `mp`, then releases it.
pub fn release_with_titles(mp: &Path, table: &[(String, String, u64)]) -> io::Result<()> {
    let prefix = format!("{}/", mp.display());
    let mut tids: Vec<&str> = table.iter().filter_map(|(on, from, _)| {
        let tid = on.strip_prefix("/system_ex/app/").filter(|t| crate::scan::valid_tid(t))?;
        let from = from.strip_prefix("<above>:").unwrap_or(from);
        (from == mp.to_string_lossy() || from.starts_with(&prefix)).then_some(tid)
    }).collect();
    tids.sort_unstable();
    tids.dedup();
    for tid in tids {
        crate::register::unmount_title(tid)?;
    }
    release(mp)
}

/// After a scan: releases every image mount not in `keep` (its image is gone, moved, or no longer
/// a source), with the titles mounted on it. A failure (busy) is logged and retried next pass.
pub fn release_unused(keep: &HashSet<PathBuf>) {
    let unused: Vec<PathBuf> = mounts().into_iter().map(|(mp, _)| mp).filter(|mp| !keep.contains(mp)).collect();
    if unused.is_empty() {
        return;
    }
    let table = match crate::web::mount_table() {
        Ok(t) => t,
        Err(e) => {
            log!("images: getfsstat: {e}");
            return;
        }
    };
    for mp in unused {
        if let Err(e) = release_with_titles(&mp, &table) {
            log!("{}: release: {e}", mp.display());
        }
    }
}
