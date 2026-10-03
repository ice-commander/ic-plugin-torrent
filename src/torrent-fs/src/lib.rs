use ic_plugin_api::{
    check_host, needs_up_to, HostCheck, IcHost, IC_ABI_VERSION, IC_ERR_HOST_TOO_OLD,
    IC_ERR_HOST_UNKNOWN, IC_ERR_INIT_FAILED, IC_OK,
};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};

pub mod cleanup;
pub mod downloads;
pub mod engine;
pub mod error;
pub mod i18n;
pub mod mount;
pub mod rows;
pub mod selection;
pub mod ticker;
pub mod torrent_rpc;
pub mod torrent_session;

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../version.rs"));

ic_plugin_api::declare_about!(
    "ic-torrent-fs",
    "Torrent",
    plugin_version!(),
    "Browses and downloads torrents"
);

pub const KIND: &str = "torrent";

pub const EXTENSIONS: &str = ".torrent";

static HOST: AtomicUsize = AtomicUsize::new(0);

fn host() -> *const IcHost {
    HOST.load(Ordering::Relaxed) as *const IcHost
}

pub(crate) fn note(said: &str) {
    let host = host();
    if host.is_null() {
        return;
    }
    if let Ok(line) = CString::new(format!("torrent: {said}")) {
        unsafe { ((*host).log_warn)(line.as_ptr()) };
    }
}

pub(crate) fn downloads_moved_on() {
    let host = host();
    if host.is_null() {
        return;
    }
    let Ok(id) = CString::new(downloads::VIEW_ID) else {
        return;
    };
    unsafe { ((*host).view_invalidate)(id.as_ptr()) };
}

pub(crate) fn listing_moved_on() {
    let host = host();
    if host.is_null() {
        return;
    }
    let Ok(extensions) = CString::new(EXTENSIONS) else {
        return;
    };
    unsafe { ((*host).fs_invalidate)(extensions.as_ptr()) };
}

pub const VIEW_ALL: &str = "torrent.view_all";
pub const VIEW_DOWNLOADED: &str = "torrent.view_downloaded";
pub const VIEW_PEERS: &str = "torrent.view_peers";
pub const START: &str = "torrent.start";
pub const STOP: &str = "torrent.stop";
pub const CLEANUP: &str = "torrent.cleanup";
pub const DELETE: &str = "torrent.delete";

const ACTIONS: &[(&str, &str, &str, u32)] = &[
    (
        VIEW_ALL,
        include_str!("../assets/list-icons.svg"),
        "torrent.view_all",
        ic_plugin_api::IC_ACTION_TOGGLE,
    ),
    (
        VIEW_DOWNLOADED,
        include_str!("../assets/download.svg"),
        "torrent.view_downloaded",
        ic_plugin_api::IC_ACTION_TOGGLE,
    ),
    (
        VIEW_PEERS,
        include_str!("../assets/peers.svg"),
        "torrent.view_peers",
        ic_plugin_api::IC_ACTION_TOGGLE,
    ),
    (
        DELETE,
        include_str!("../assets/delete-file.svg"),
        "torrent.delete",
        ic_plugin_api::IC_ACTION_BUTTON,
    ),
    (
        START,
        include_str!("../assets/play.svg"),
        "torrent.start",
        ic_plugin_api::IC_ACTION_BUTTON,
    ),
    (
        STOP,
        include_str!("../assets/stop.svg"),
        "torrent.stop",
        ic_plugin_api::IC_ACTION_BUTTON,
    ),
    (
        CLEANUP,
        include_str!("../assets/clean.svg"),
        "torrent.cleanup",
        ic_plugin_api::IC_ACTION_BUTTON,
    ),
];

#[cfg_attr(feature = "export-abi", no_mangle)]
pub extern "C" fn ic_plugin_init(host: *const IcHost, _kind: *const c_char) -> c_int {
    // The mount reads the `.torrent` through the host's fs_* calls.
    match check_host(
        host,
        IC_ABI_VERSION,
        needs_up_to(std::mem::offset_of!(IcHost, fs_local_path)),
    ) {
        HostCheck::Ok => {}
        HostCheck::WrongMagic => return IC_ERR_HOST_UNKNOWN,
        HostCheck::TooOld { .. } | HostCheck::Truncated { .. } => return IC_ERR_HOST_TOO_OLD,
    }
    HOST.store(host as usize, Ordering::Relaxed);

    // A cell is plain text, not a key the host could translate.
    let spoken = unsafe { ((*host).language)() };
    if !spoken.is_null() {
        i18n::speaks(&unsafe { CStr::from_ptr(spoken) }.to_string_lossy());
    }
    for (language, catalogue) in i18n::LOCALES {
        let Ok(tag) = CString::new(*language) else {
            continue;
        };
        unsafe {
            ((*host).register_locales)(tag.as_ptr(), catalogue.as_ptr(), catalogue.len() as u64)
        };
    }

    let Ok(extensions) = CString::new(EXTENSIONS) else {
        return IC_ERR_INIT_FAILED;
    };
    static TABLE: std::sync::OnceLock<ic_plugin_api::IcFsVTable> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(mount::vtable);
    let registered =
        unsafe { ((*host).register_filesystem)(extensions.as_ptr(), table, std::ptr::null_mut()) };
    if registered != IC_OK {
        return registered;
    }

    if let (Ok(id), Ok(svg)) = (
        CString::new(downloads::BUTTON_ID),
        CString::new(downloads::ICON),
    ) {
        unsafe {
            ((*host).add_header_button)(
                id.as_ptr(),
                svg.as_ptr(),
                c"".as_ptr(),
                c"torrent.downloads_heading".as_ptr(),
                ic_plugin_api::IC_SIDE_RIGHT,
                0,
                on_indicator,
                std::ptr::null_mut(),
            );
            ((*host).set_header_visible)(id.as_ptr(), 0);
        }
        std::mem::forget(id);
    }

    if let Ok(id) = CString::new(downloads::VIEW_ID) {
        static DOWNLOADS: std::sync::OnceLock<ic_plugin_api::IcViewVTable> =
            std::sync::OnceLock::new();
        let view = DOWNLOADS.get_or_init(downloads::vtable);
        unsafe { ((*host).register_view)(id.as_ptr(), id.as_ptr(), view, std::ptr::null_mut()) };
        std::mem::forget(id);
    }

    // The panel's own buttons write into a folder, and a torrent is not one.
    unsafe { ((*host).set_default_toolbar_visible)(extensions.as_ptr(), 0) };

    let Ok(view_id) = CString::new(cleanup::VIEW_ID) else {
        return IC_ERR_INIT_FAILED;
    };
    static VIEW: std::sync::OnceLock<ic_plugin_api::IcViewVTable> = std::sync::OnceLock::new();
    let view = VIEW.get_or_init(cleanup::vtable);
    unsafe {
        ((*host).register_view)(
            view_id.as_ptr(),
            view_id.as_ptr(),
            view,
            std::ptr::null_mut(),
        )
    };
    std::mem::forget(view_id);

    for (id, svg, tooltip, kind) in ACTIONS {
        let (Ok(id), Ok(svg), Ok(tooltip)) = (
            CString::new(*id),
            CString::new(*svg),
            CString::new(*tooltip),
        ) else {
            continue;
        };
        unsafe {
            ((*host).register_fs_action)(
                extensions.as_ptr(),
                id.as_ptr(),
                svg.as_ptr(),
                tooltip.as_ptr(),
                ic_plugin_api::IC_ENABLE_ALWAYS,
                *kind,
                on_action,
                id.as_ptr() as *mut c_void,
            )
        };
        // The host keeps this pointer for as long as the plugin is loaded.
        std::mem::forget(id);
    }
    IC_OK
}

extern "C" fn on_indicator(_user_data: *mut c_void, _parent: *mut c_void) {
    let host = host();
    if host.is_null() {
        return;
    }
    let Ok(id) = CString::new(downloads::VIEW_ID) else {
        return;
    };
    unsafe { ((*host).open_view)(id.as_ptr(), std::ptr::null(), 0) };
}

pub(crate) fn show_what_is_downloading() {
    let host = host();
    if host.is_null() {
        return;
    }
    let Ok(id) = CString::new(downloads::BUTTON_ID) else {
        return;
    };
    match torrent_session::activity() {
        Some(activity) => {
            if let Ok(said) = CString::new(downloads::speeds(&activity)) {
                unsafe { ((*host).set_header_label)(id.as_ptr(), said.as_ptr()) };
            }
            unsafe { ((*host).set_header_visible)(id.as_ptr(), 1) };
        }
        None => unsafe {
            ((*host).set_header_visible)(id.as_ptr(), 0);
        },
    }
}

pub fn in_the_background<F>(work: impl FnOnce() -> F + Send + 'static)
where
    F: std::future::Future<Output = Result<(), error::AppError>>,
{
    let Some(runtime) = engine::handle() else {
        return;
    };
    ticker::took_on_work();
    std::thread::spawn(move || {
        if let Err(why) = runtime.block_on(work()) {
            note(&why.to_string());
        }
        listing_moved_on();
        downloads_moved_on();
        show_what_is_downloading();
        ticker::finished_work();
    });
}

fn throw_away_what_is_selected(mount: ic_plugin_api::IcFsHandle, path: &str) {
    let host = host();
    if host.is_null() {
        return;
    }
    let Some(rpc) = mount::mounted_rpc(mount) else {
        return;
    };
    let picked = what_is_selected();
    if picked.is_empty() {
        note("nothing is selected, so nothing was thrown away");
        return;
    }
    let mut thrown = 0usize;
    for one in picked {
        let Some(inside) = selection::inside(path, &one) else {
            continue;
        };
        let (dir, name) = match inside.rsplit_once('/') {
            Some((dir, name)) => (dir.to_string(), name.to_string()),
            None => (String::new(), inside),
        };
        match rpc.delete(&dir, &name) {
            Ok(()) => thrown += 1,
            Err(why) => note(&why.to_string()),
        }
    }
    if thrown > 0 {
        listing_moved_on();
    }
}

/// Never `None`: librqbit reads `only_files: None` as every file.
fn wanted_files(path: &str) -> Option<Vec<usize>> {
    Some(
        torrent_session::read_state(path)
            .map(|held| held.selected)
            .unwrap_or_default(),
    )
}

fn what_is_selected() -> Vec<String> {
    let host = host();
    if host.is_null() {
        return Vec::new();
    }
    unsafe { ((*host).selection)() }
        .as_slice()
        .iter()
        .filter_map(|item| item.path_string())
        .collect()
}

pub fn anything_wanted(path: &str) -> bool {
    !matches!(wanted_files(path), Some(chosen) if chosen.is_empty())
}

pub(crate) fn start_now(mount: ic_plugin_api::IcFsHandle) {
    let (Some(path), Some(bytes)) = (mount::torrent_path(mount), mount::torrent_bytes(mount))
    else {
        return;
    };
    if !anything_wanted(&path) {
        note("nothing is ticked, so there is nothing to start");
        return;
    }
    ticker::wake();
    let selected = wanted_files(&path);
    in_the_background(move || async move { torrent_session::start(&path, bytes, selected).await });
}

fn fetch_what_is_selected(mount: ic_plugin_api::IcFsHandle, _path: &str) {
    change_what_is_fetched(mount, true)
}

fn leave_what_is_selected(mount: ic_plugin_api::IcFsHandle, _path: &str) {
    change_what_is_fetched(mount, false)
}

fn change_what_is_fetched(mount: ic_plugin_api::IcFsHandle, wanted: bool) {
    let Some(rpc) = mount::mounted_rpc(mount) else {
        return;
    };
    let picked = what_is_selected();
    if picked.is_empty() {
        note("nothing is selected, so nothing changed");
        return;
    }
    match rpc.fetch_selected(&picked, wanted) {
        Ok(0) => note("none of what is selected is in this torrent"),
        Ok(how_many) => {
            note(&format!(
                "{how_many} of the torrent's files {}",
                if wanted { "wanted" } else { "left alone" }
            ));
            listing_moved_on();
        }
        Err(why) => note(&why.to_string()),
    }
}

fn ask_before_deleting(path: &str) {
    let host = host();
    if host.is_null() {
        return;
    }
    let carried = cleanup::about(path);
    let Ok(id) = CString::new(cleanup::VIEW_ID) else {
        return;
    };
    unsafe { ((*host).open_view)(id.as_ptr(), carried.as_ptr(), carried.len() as u64) };
}

extern "C" fn on_action(
    mount: ic_plugin_api::IcFsHandle,
    user_data: *mut c_void,
    _parent: *mut c_void,
) {
    if user_data.is_null() {
        return;
    }
    let which = unsafe { CStr::from_ptr(user_data as *const c_char) }
        .to_string_lossy()
        .to_string();
    let Some(path) = mount::torrent_path(mount) else {
        note("a torrent button was pressed with no torrent open");
        return;
    };
    match which.as_str() {
        VIEW_ALL | VIEW_DOWNLOADED | VIEW_PEERS => {
            let Some(rpc) = mount::mounted_rpc(mount) else {
                return;
            };
            rpc.set_view(match which.as_str() {
                VIEW_DOWNLOADED => torrent_rpc::TorrentView::Downloaded,
                VIEW_PEERS => torrent_rpc::TorrentView::Peers,
                _ => torrent_rpc::TorrentView::All,
            });
            listing_moved_on();
        }
        DELETE => throw_away_what_is_selected(mount, &path),
        START => fetch_what_is_selected(mount, &path),
        STOP => leave_what_is_selected(mount, &path),
        CLEANUP => ask_before_deleting(&path),
        other => note(&format!("no such button: {other}")),
    }
}

#[cfg_attr(feature = "export-abi", no_mangle)]
pub extern "C" fn ic_plugin_shutdown() {
    // Order matters: the ticker, then the torrents, then the runtime they run on.
    ticker::settle(torrent_session::CLOSE_GRACE);
    torrent_session::shutdown_blocking(torrent_session::CLOSE_GRACE);
    engine::stop(torrent_session::CLOSE_GRACE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_says_what_it_is_before_it_is_initialised() {
        let about = ic_plugin_about();
        assert!(!about.is_null());
        let name = unsafe { std::ffi::CStr::from_ptr(ic_plugin_name()) }
            .to_string_lossy()
            .to_string();
        let version = unsafe { std::ffi::CStr::from_ptr(ic_plugin_version()) }
            .to_string_lossy()
            .to_string();
        assert_eq!(name, "Torrent");
        assert_eq!(version, plugin_version!());
    }

    #[test]
    fn a_torrent_nobody_chose_files_for_fetches_none_of_them() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir
            .path()
            .join("fresh.torrent")
            .to_string_lossy()
            .to_string();
        assert_eq!(wanted_files(&path), Some(Vec::new()));
        assert!(!anything_wanted(&path));

        torrent_session::write_state(
            &path,
            &torrent_session::TorrentState {
                info_hash: "abc".to_string(),
                selected: Vec::new(),
                done: Vec::new(),
            },
        )
        .expect("written");
        assert!(!anything_wanted(&path));
    }

    #[test]
    fn a_torrent_someone_chose_files_for_fetches_those() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir
            .path()
            .join("chosen.torrent")
            .to_string_lossy()
            .to_string();
        torrent_session::write_state(
            &path,
            &torrent_session::TorrentState {
                info_hash: "abc".to_string(),
                selected: vec![0, 2],
                done: Vec::new(),
            },
        )
        .expect("written");
        assert_eq!(wanted_files(&path), Some(vec![0, 2]));
    }

    #[test]
    fn shutting_down_without_ever_starting_a_torrent_is_harmless() {
        let _held = crate::ticker::one_at_a_time();
        ic_plugin_shutdown();
        assert!(!torrent_session::is_open("/nowhere/none.torrent"));
    }

    #[test]
    fn shutting_down_leaves_nothing_of_ours_running() {
        let _held = crate::ticker::one_at_a_time();
        ticker::wake();
        ic_plugin_shutdown();
        assert!(!ticker::is_running());
        assert_eq!(ticker::still_busy(), 0);
    }

    #[test]
    fn it_refuses_a_host_it_does_not_recognise() {
        assert_eq!(
            ic_plugin_init(std::ptr::null(), c"gtk".as_ptr()),
            IC_ERR_HOST_UNKNOWN
        );
    }
}
