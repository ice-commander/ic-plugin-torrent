//! Blocking is fine here: the host calls into a mount on a worker thread.

use crate::rows::RemoteFileEntry;
use crate::torrent_rpc::TorrentFileSystemRpc;
use ic_plugin_api::{
    IcBytes, IcColumns, IcDirEntry, IcFsColumn, IcFsHandle, IcFsSource, IcFsVTable, IcListing,
    IcRow, IcRows, IC_ACTION_ENABLED, IC_ACTION_ON, IC_ACTION_SHOWN, IC_COLUMN_CHECK,
    IC_COLUMN_TEXT, IC_OPEN_READ, IC_SEEK_END, IC_SEEK_SET,
};
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

struct Mounted {
    rpc: TorrentFileSystemRpc,
    trouble: RefCell<String>,
    // Everything below stays alive while the host reads what was answered.
    names: RefCell<Vec<CString>>,
    entries: RefCell<Vec<IcDirEntry>>,
    cells: RefCell<Vec<CString>>,
    cell_ptrs: RefCell<Vec<*const c_char>>,
    rows: RefCell<Vec<IcRow>>,
    column_text: RefCell<Vec<CString>>,
    columns: RefCell<Vec<IcFsColumn>>,
    bytes: RefCell<Vec<u8>>,
    said: RefCell<CString>,
}

fn held(handle: IcFsHandle) -> Option<&'static Mounted> {
    if handle.is_null() {
        return None;
    }
    Some(unsafe { &*(handle as *const Mounted) })
}

fn path_of(path: *const c_char) -> String {
    if path.is_null() {
        return "/".to_string();
    }
    unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .to_string()
}

fn blamed(mounted: &Mounted, why: String) {
    crate::note(&why);
    *mounted.trouble.borrow_mut() = why;
}

pub fn torrent_path(handle: IcFsHandle) -> Option<String> {
    held(handle).map(|m| m.rpc.torrent_path().to_string())
}

pub fn mounted_rpc(handle: IcFsHandle) -> Option<&'static TorrentFileSystemRpc> {
    held(handle).map(|m| &m.rpc)
}

pub fn torrent_bytes(handle: IcFsHandle) -> Option<Vec<u8>> {
    held(handle).map(|m| m.rpc.bytes.clone())
}

fn read_whole(source: IcFsSource, named: &CStr) -> Option<Vec<u8>> {
    let host = crate::host();
    if host.is_null() {
        return None;
    }
    let stream = unsafe { ((*host).fs_open)(source, named.as_ptr(), IC_OPEN_READ) };
    if stream.is_null() {
        return None;
    }
    let end = unsafe { ((*host).fs_seek)(stream, 0, IC_SEEK_END) };
    unsafe { ((*host).fs_seek)(stream, 0, IC_SEEK_SET) };
    let mut held = Vec::with_capacity(end.max(0) as usize);
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = unsafe { ((*host).fs_read)(stream, buffer.as_mut_ptr(), buffer.len() as u64) };
        if read <= 0 {
            break;
        }
        held.extend_from_slice(&buffer[..read as usize]);
    }
    unsafe { ((*host).fs_close)(stream) };
    Some(held)
}

fn local_path(source: IcFsSource, named: &CStr) -> Option<String> {
    let host = crate::host();
    if host.is_null() {
        return None;
    }
    let answered = unsafe { ((*host).fs_local_path)(source, named.as_ptr()) };
    if answered.is_null() {
        return None;
    }
    // The host's answer is only good until the next call on this thread.
    Some(
        unsafe { CStr::from_ptr(answered) }
            .to_string_lossy()
            .into_owned(),
    )
}

extern "C" fn open_in(
    source: IcFsSource,
    name: *const c_char,
    _user_data: *mut c_void,
) -> IcFsHandle {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    let named = unsafe { CStr::from_ptr(name) }.to_owned();
    let Some(path) = local_path(source, &named) else {
        crate::note(
            "the torrent file is nowhere on this machine, so there is nowhere to download to",
        );
        return std::ptr::null_mut();
    };
    let raw = read_whole(source, &named).unwrap_or_default();
    if raw.is_empty() {
        crate::note("the torrent file could not be read");
        return std::ptr::null_mut();
    }
    let mounted = Box::new(Mounted {
        rpc: TorrentFileSystemRpc::new(path, raw),
        trouble: RefCell::new(String::new()),
        names: RefCell::new(Vec::new()),
        entries: RefCell::new(Vec::new()),
        cells: RefCell::new(Vec::new()),
        cell_ptrs: RefCell::new(Vec::new()),
        rows: RefCell::new(Vec::new()),
        column_text: RefCell::new(Vec::new()),
        columns: RefCell::new(Vec::new()),
        bytes: RefCell::new(Vec::new()),
        said: RefCell::new(CString::default()),
    });
    let handle = Box::into_raw(mounted) as IcFsHandle;
    crate::start_now(handle);
    handle
}

extern "C" fn close(handle: IcFsHandle) {
    if handle.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(handle as *mut Mounted) });
}

fn entries_of(mounted: &Mounted, listed: &[RemoteFileEntry]) -> usize {
    let names: Vec<CString> = listed
        .iter()
        .map(|entry| CString::new(entry.name.clone()).unwrap_or_default())
        .collect();
    let entries: Vec<IcDirEntry> = listed
        .iter()
        .zip(names.iter())
        .map(|(entry, name)| IcDirEntry {
            name: name.as_ptr(),
            is_dir: i32::from(entry.is_dir),
            size: entry.size,
            modified: entry.modified,
            permissions: entry.permissions.unwrap_or(0),
            has_permissions: i32::from(entry.permissions.is_some()),
        })
        .collect();
    let count = entries.len();
    *mounted.names.borrow_mut() = names;
    *mounted.entries.borrow_mut() = entries;
    count
}

extern "C" fn list(handle: IcFsHandle, path: *const c_char) -> IcListing {
    let Some(mounted) = held(handle) else {
        return IcListing::EMPTY;
    };
    match mounted.rpc.list_dir(path_of(path)) {
        Ok(listed) => {
            entries_of(mounted, &listed);
            let borrowed = mounted.entries.borrow();
            IcListing {
                items: borrowed.as_ptr(),
                count: borrowed.len() as u32,
            }
        }
        Err(why) => {
            blamed(mounted, why.to_string());
            IcListing::EMPTY
        }
    }
}

extern "C" fn list_rows(handle: IcFsHandle, path: *const c_char) -> IcRows {
    let Some(mounted) = held(handle) else {
        return IcRows::EMPTY;
    };
    let listed = match mounted.rpc.list_dir(path_of(path)) {
        Ok(listed) => listed,
        Err(why) => {
            blamed(mounted, why.to_string());
            return IcRows::EMPTY;
        }
    };
    entries_of(mounted, &listed);

    let cells: Vec<CString> = listed
        .iter()
        .flat_map(|entry| entry.extra.iter())
        .map(|text| CString::new(text.clone()).unwrap_or_default())
        .collect();
    let pointers: Vec<*const c_char> = cells.iter().map(|text| text.as_ptr()).collect();
    *mounted.cells.borrow_mut() = cells;
    *mounted.cell_ptrs.borrow_mut() = pointers;

    let entries = mounted.entries.borrow();
    let pointers = mounted.cell_ptrs.borrow();
    let mut at = 0usize;
    let rows: Vec<IcRow> = entries
        .iter()
        .zip(listed.iter())
        .map(|(entry, source)| {
            let count = source.extra.len();
            let extra = if count == 0 {
                std::ptr::null()
            } else {
                // SAFETY: the cells were flattened from these rows in order, so at + count <= pointers.len().
                unsafe { pointers.as_ptr().add(at) }
            };
            at += count;
            IcRow {
                entry: *entry,
                extra,
                extra_count: count as u32,
            }
        })
        .collect();
    drop(entries);
    drop(pointers);
    *mounted.rows.borrow_mut() = rows;
    let borrowed = mounted.rows.borrow();
    IcRows {
        count: borrowed.len() as u32,
        items: borrowed.as_ptr(),
        ..IcRows::EMPTY
    }
}

extern "C" fn columns(handle: IcFsHandle) -> IcColumns {
    let Some(mounted) = held(handle) else {
        return IcColumns::EMPTY;
    };
    let declared = mounted.rpc.extra_columns();
    let mut text: Vec<CString> = Vec::with_capacity(declared.len() * 2);
    for spec in &declared {
        text.push(CString::new(spec.key.clone()).unwrap_or_default());
        text.push(CString::new(spec.title.clone()).unwrap_or_default());
    }
    let built: Vec<IcFsColumn> = declared
        .iter()
        .enumerate()
        .map(|(at, spec)| IcFsColumn {
            key: text[at * 2].as_ptr(),
            title: text[at * 2 + 1].as_ptr(),
            width: spec.width.unwrap_or(0),
            kind: match spec.kind {
                crate::rows::ColumnKind::Check => IC_COLUMN_CHECK,
                crate::rows::ColumnKind::Text => IC_COLUMN_TEXT,
            },
        })
        .collect();
    *mounted.column_text.borrow_mut() = text;
    *mounted.columns.borrow_mut() = built;
    let borrowed = mounted.columns.borrow();
    IcColumns {
        count: borrowed.len() as u32,
        items: borrowed.as_ptr(),
        ..IcColumns::EMPTY
    }
}

extern "C" fn read(handle: IcFsHandle, path: *const c_char) -> IcBytes {
    let Some(mounted) = held(handle) else {
        return IcBytes::EMPTY;
    };
    match mounted.rpc.read_file(path_of(path)) {
        Ok(bytes) => {
            *mounted.bytes.borrow_mut() = bytes;
            let borrowed = mounted.bytes.borrow();
            IcBytes {
                data: borrowed.as_ptr(),
                len: borrowed.len() as u64,
            }
        }
        Err(why) => {
            blamed(mounted, why.to_string());
            IcBytes::EMPTY
        }
    }
}

extern "C" fn action_state(handle: IcFsHandle, action_id: *const c_char) -> u32 {
    let Some(mounted) = held(handle) else {
        return ic_plugin_api::IC_ACTION_DEFAULT;
    };
    let which = if action_id.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(action_id) }
            .to_string_lossy()
            .to_string()
    };
    let path = mounted.rpc.torrent_path();
    let running = crate::torrent_session::is_active(path);
    let showing = mounted.rpc.view();

    let view = |wanted: crate::torrent_rpc::TorrentView| {
        if showing == wanted {
            IC_ACTION_SHOWN | IC_ACTION_ON
        } else {
            IC_ACTION_SHOWN | IC_ACTION_ENABLED
        }
    };
    let view_while_running = |wanted: crate::torrent_rpc::TorrentView| {
        if !running && showing != wanted {
            return IC_ACTION_SHOWN;
        }
        view(wanted)
    };

    match which.as_str() {
        crate::VIEW_ALL => view(crate::torrent_rpc::TorrentView::All),
        crate::VIEW_DOWNLOADED => view_while_running(crate::torrent_rpc::TorrentView::Downloaded),
        crate::VIEW_PEERS => view_while_running(crate::torrent_rpc::TorrentView::Peers),
        crate::START | crate::STOP | crate::DELETE
            if showing == crate::torrent_rpc::TorrentView::Peers =>
        {
            IC_ACTION_SHOWN
        }
        crate::START | crate::STOP | crate::DELETE => IC_ACTION_SHOWN | IC_ACTION_ENABLED,
        // Cleaning up while it runs would fight the session for the files.
        crate::CLEANUP if running => IC_ACTION_SHOWN,
        crate::CLEANUP => IC_ACTION_SHOWN | IC_ACTION_ENABLED,
        _ => ic_plugin_api::IC_ACTION_DEFAULT,
    }
}

extern "C" fn cell_clicked(
    handle: IcFsHandle,
    dir: *const c_char,
    name: *const c_char,
    column_key: *const c_char,
    ticked: c_int,
) -> c_int {
    let Some(mounted) = held(handle) else {
        return ic_plugin_api::IC_ERR_INIT_FAILED;
    };
    let column = path_of(column_key);
    if column != crate::torrent_rpc::FETCH_COLUMN {
        return ic_plugin_api::IC_ERR_INIT_FAILED;
    }
    match mounted
        .rpc
        .toggle_fetch(&path_of(dir), &path_of(name), ticked != 0)
    {
        Ok(()) => ic_plugin_api::IC_OK,
        Err(why) => {
            blamed(mounted, why.to_string());
            ic_plugin_api::IC_ERR_INIT_FAILED
        }
    }
}

/// Left as `None`, these slots would get the host's own refusal, which calls the filesystem read-only.
fn refuse(mounted: &Mounted) -> c_int {
    blamed(
        mounted,
        crate::torrent_rpc::TorrentFileSystemRpc::cannot_be_written_to().to_string(),
    );
    ic_plugin_api::IC_ERR_INIT_FAILED
}

extern "C" fn write(handle: IcFsHandle, _: *const c_char, _: *const u8, _: u64) -> c_int {
    match held(handle) {
        Some(mounted) => refuse(mounted),
        None => ic_plugin_api::IC_ERR_INIT_FAILED,
    }
}

extern "C" fn create_dir(handle: IcFsHandle, _: *const c_char) -> c_int {
    match held(handle) {
        Some(mounted) => refuse(mounted),
        None => ic_plugin_api::IC_ERR_INIT_FAILED,
    }
}

extern "C" fn rename(handle: IcFsHandle, _: *const c_char, _: *const c_char) -> c_int {
    match held(handle) {
        Some(mounted) => refuse(mounted),
        None => ic_plugin_api::IC_ERR_INIT_FAILED,
    }
}

extern "C" fn remove(handle: IcFsHandle, path: *const c_char) -> c_int {
    let Some(mounted) = held(handle) else {
        return ic_plugin_api::IC_ERR_INIT_FAILED;
    };
    let whole = path_of(path).replace('\\', "/");
    let trimmed = whole.trim_matches('/');
    let (dir, name) = match trimmed.rsplit_once('/') {
        Some((dir, name)) => (dir, name),
        None => ("", trimmed),
    };
    if name.is_empty() {
        blamed(mounted, "there is no such file in this torrent".to_string());
        return ic_plugin_api::IC_ERR_INIT_FAILED;
    }
    match mounted.rpc.delete(dir, name) {
        Ok(()) => ic_plugin_api::IC_OK,
        Err(why) => {
            blamed(mounted, why.to_string());
            ic_plugin_api::IC_ERR_INIT_FAILED
        }
    }
}

extern "C" fn is_read_only(handle: IcFsHandle) -> c_int {
    c_int::from(held(handle).map(|m| m.rpc.is_read_only()).unwrap_or(true))
}

extern "C" fn last_error(handle: IcFsHandle) -> *const c_char {
    let Some(mounted) = held(handle) else {
        return std::ptr::null();
    };
    let said = CString::new(mounted.trouble.borrow().clone()).unwrap_or_default();
    *mounted.said.borrow_mut() = said;
    mounted.said.borrow().as_ptr()
}

pub fn vtable() -> IcFsVTable {
    IcFsVTable {
        struct_size: std::mem::size_of::<IcFsVTable>() as u32,
        open_in,
        close,
        list,
        read,
        is_read_only,
        last_error,
        write: Some(write),
        create_dir: Some(create_dir),
        remove: Some(remove),
        rename: Some(rename),
        shell_open: None,
        shell_read: None,
        shell_write: None,
        shell_resize: None,
        shell_close: None,
        shell_available: None,
        columns: Some(columns),
        list_rows: Some(list_rows),
        action_state: Some(action_state),
        cell_clicked: Some(cell_clicked),
        set_permissions: None,
    }
}
