use crate::error::AppError;
use crate::rows::{ColumnSpec, RemoteFileEntry};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TorrentEntry {
    pub id: usize,
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TorrentContents {
    pub name: String,
    pub entries: Vec<TorrentEntry>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TorrentView {
    #[default]
    All,
    Downloaded,
    Peers,
}

pub struct TorrentFileSystemRpc {
    pub relative_path_in_parent: String,
    pub bytes: Vec<u8>,
    pub contents: Rc<RefCell<Option<TorrentContents>>>,
    pub view: Rc<std::cell::Cell<TorrentView>>,
}

impl TorrentFileSystemRpc {
    pub fn new(relative_path_in_parent: String, bytes: Vec<u8>) -> Self {
        crate::torrent_session::acquire(&relative_path_in_parent);
        Self {
            relative_path_in_parent,
            bytes,
            contents: Rc::new(RefCell::new(None)),
            view: Rc::new(std::cell::Cell::new(TorrentView::All)),
        }
    }

    pub fn torrent_path(&self) -> &str {
        &self.relative_path_in_parent
    }

    pub fn view(&self) -> TorrentView {
        self.view.get()
    }

    pub fn set_view(&self, view: TorrentView) {
        self.view.set(view);
    }
}

pub const FETCH_COLUMN: &str = "torrent_fetch";

pub fn status_text(done_bytes: u64, total_bytes: u64) -> String {
    if total_bytes > 0 && done_bytes >= total_bytes {
        return crate::i18n::tr("torrent.status_ready");
    }
    if done_bytes == 0 {
        return String::new();
    }
    let percent = (done_bytes as f64 / total_bytes as f64 * 100.0).floor() as u64;
    format!("{}%", percent.min(99))
}

impl Drop for TorrentFileSystemRpc {
    fn drop(&mut self) {
        crate::torrent_session::release(&self.relative_path_in_parent);
    }
}

pub fn downloaded_rows(
    entries: &[TorrentEntry],
    is_complete: &dyn Fn(usize, u64) -> bool,
) -> Vec<RemoteFileEntry> {
    let mut rows: Vec<RemoteFileEntry> = entries
        .iter()
        .filter(|e| is_complete(e.id, e.size))
        .map(|e| RemoteFileEntry {
            name: e.path.clone(),
            is_dir: false,
            size: e.size,
            modified: 0,
            permissions: None,
            extra: Vec::new(),
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

pub fn peer_rows(peers: &[crate::torrent_session::PeerRow]) -> Vec<RemoteFileEntry> {
    peers
        .iter()
        .map(|p| RemoteFileEntry {
            name: p.addr.clone(),
            is_dir: false,
            size: p.fetched_bytes,
            modified: 0,
            permissions: None,
            extra: vec![p.state.clone(), p.pieces.to_string(), p.errors.to_string()],
        })
        .collect()
}

pub fn parse_torrent(data: &[u8]) -> Result<TorrentContents, AppError> {
    let meta = librqbit::torrent_from_bytes::<librqbit::ByteBufOwned>(data)
        .map_err(|e| AppError::Other(format!("not a valid torrent file: {}", e)))?;
    let name = meta
        .info
        .name
        .as_ref()
        .map(|n| String::from_utf8_lossy(n.as_ref()).to_string())
        .unwrap_or_default();
    let details = meta
        .info
        .iter_file_details()
        .map_err(|e| AppError::Other(format!("torrent metadata is unusable: {}", e)))?;
    let mut entries = Vec::new();
    for (id, d) in details.enumerate() {
        if d.attrs().padding {
            continue;
        }
        let parts = d
            .filename
            .to_vec()
            .map_err(|e| AppError::Other(format!("torrent has an unreadable file name: {}", e)))?;
        let path = parts.join("/");
        if path.is_empty() {
            continue;
        }
        entries.push(TorrentEntry {
            id,
            path,
            size: d.len,
        });
    }
    Ok(TorrentContents { name, entries })
}

pub fn normalize_internal_path(path: &str) -> String {
    path.replace('\\', "/").trim_matches('/').to_string()
}

pub fn list_level(entries: &[TorrentEntry], internal_dir: &str) -> Vec<RemoteFileEntry> {
    let dir = normalize_internal_path(internal_dir);
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{}/", dir)
    };

    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<(String, u64)> = Vec::new();

    for entry in entries {
        let rest = match entry.path.strip_prefix(prefix.as_str()) {
            Some(r) if !prefix.is_empty() => r,
            _ if prefix.is_empty() => entry.path.as_str(),
            _ => continue,
        };
        let mut parts = rest.splitn(2, '/');
        let head = match parts.next() {
            Some(h) if !h.is_empty() => h,
            _ => continue,
        };
        if parts.next().is_some() {
            if !dirs.iter().any(|d| d == head) {
                dirs.push(head.to_string());
            }
        } else if !files.iter().any(|(n, _)| n == head) {
            files.push((head.to_string(), entry.size));
        }
    }

    files.retain(|(n, _)| !dirs.iter().any(|d| d == n));

    let mut out: Vec<RemoteFileEntry> = dirs
        .into_iter()
        .map(|name| RemoteFileEntry {
            name,
            is_dir: true,
            size: 0,
            modified: 0,
            permissions: Some(0o755),
            extra: Vec::new(),
        })
        .chain(files.into_iter().map(|(name, size)| RemoteFileEntry {
            name,
            is_dir: false,
            size,
            modified: 0,
            permissions: None,
            extra: Vec::new(),
        }))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

impl TorrentFileSystemRpc {
    fn ensure_loaded(&self) -> Result<TorrentContents, AppError> {
        if let Some(cached) = self.contents.borrow().clone() {
            return Ok(cached);
        }
        let parsed = parse_torrent(&self.bytes)?;
        *self.contents.borrow_mut() = Some(parsed.clone());
        Ok(parsed)
    }

    pub fn toggle_fetch(&self, dir: &str, name: &str, ticked: bool) -> Result<(), AppError> {
        let contents = self.ensure_loaded()?;
        let path = self.relative_path_in_parent.clone();
        let stands_for = crate::selection::ids_for_row(&contents.entries, dir, name);
        if stands_for.is_empty() {
            return Err(AppError::Other(
                "that row is not a file of this torrent".to_string(),
            ));
        }
        let held = crate::torrent_session::read_state(&path);
        let chosen = crate::selection::spelled_out(&contents.entries, held.as_ref());
        let next = crate::selection::after_click(&chosen, &stands_for, ticked);
        let _ = held;
        crate::torrent_session::amend_state(&path, |state| {
            state.selected = next.clone();
        })?;
        if crate::torrent_session::is_active(&path) {
            crate::in_the_background(move || async move {
                let wanted: std::collections::HashSet<usize> = next.into_iter().collect();
                crate::torrent_session::set_selection(&path, &wanted).await
            });
        }
        self.follow_what_is_wanted();
        Ok(())
    }

    fn remember_what_is_downloaded(
        &self,
        contents: &TorrentContents,
        stats: &librqbit::TorrentStats,
    ) {
        let done: Vec<usize> = contents
            .entries
            .iter()
            .filter(|entry| {
                entry.size > 0
                    && stats.file_progress.get(entry.id).copied().unwrap_or(0) >= entry.size
            })
            .map(|entry| entry.id)
            .collect();
        let _ = crate::torrent_session::amend_state(&self.relative_path_in_parent, |state| {
            state.done = done;
        });
    }

    pub fn follow_what_is_wanted(&self) {
        let path = self.relative_path_in_parent.clone();
        let wanted = crate::torrent_session::read_state(&path)
            .map(|held| !held.selected.is_empty())
            .unwrap_or(false);
        match (wanted, crate::torrent_session::is_active(&path)) {
            (true, false) => {
                let bytes = self.bytes.clone();
                let selected = crate::torrent_session::read_state(&path).map(|held| held.selected);
                crate::ticker::wake();
                crate::in_the_background(move || async move {
                    crate::torrent_session::start(&path, bytes, selected).await
                });
            }
            (false, true) => {
                crate::in_the_background(move || async move {
                    crate::torrent_session::stop(&path).await
                });
            }
            _ => {}
        }
    }

    pub fn fetch_selected(&self, picked: &[String], ticked: bool) -> Result<usize, AppError> {
        let contents = self.ensure_loaded()?;
        let path = self.relative_path_in_parent.clone();
        let touched = crate::selection::chosen(&contents.entries, &path, picked);
        if touched.is_empty() {
            return Ok(0);
        }
        let held = crate::torrent_session::read_state(&path);
        let chosen = crate::selection::spelled_out(&contents.entries, held.as_ref());
        let next = crate::selection::after_click(&chosen, &touched, ticked);
        crate::torrent_session::amend_state(&path, |state| {
            state.selected = next.clone();
        })?;
        if crate::torrent_session::is_active(&path) {
            crate::in_the_background(move || async move {
                let wanted: std::collections::HashSet<usize> = next.into_iter().collect();
                crate::torrent_session::set_selection(&path, &wanted).await
            });
        }
        self.follow_what_is_wanted();
        Ok(touched.len())
    }

    /// A file sharing a piece with one still wanted is written again when that piece arrives.
    pub fn delete(&self, dir: &str, name: &str) -> Result<(), AppError> {
        let contents = self.ensure_loaded()?;
        let stands_for = crate::selection::ids_for_row(&contents.entries, dir, name);
        if stands_for.is_empty() {
            return Err(AppError::Other(
                "that row is not a file of this torrent".to_string(),
            ));
        }
        // Unwanted before deleting, or a running torrent fetches it straight back.
        self.toggle_fetch(dir, name, false)?;
        crate::torrent_session::amend_state(&self.relative_path_in_parent, |state| {
            state.done.retain(|id| !stands_for.contains(id));
        })?;

        let dir = normalize_internal_path(dir);
        let mut failed: Option<AppError> = None;
        for entry in contents
            .entries
            .iter()
            .filter(|e| stands_for.contains(&e.id))
        {
            let on_disk = crate::torrent_session::downloaded_file_path(
                &self.relative_path_in_parent,
                &entry.path,
            );
            if !on_disk.exists() {
                continue;
            }
            if let Err(why) = std::fs::remove_file(&on_disk) {
                failed = Some(AppError::from(why));
            }
        }
        // TODO: remove the folder a directory row stood for once its files are gone.
        let _ = dir;
        match failed {
            Some(why) => Err(why),
            None => Ok(()),
        }
    }

    pub fn known_contents(&self) -> Option<TorrentContents> {
        self.contents.borrow().clone()
    }

    pub fn torrent_name(&self) -> Option<String> {
        self.contents.borrow().as_ref().map(|c| c.name.clone())
    }
}

impl TorrentFileSystemRpc {
    pub fn list_dir(&self, path: String) -> Result<Vec<RemoteFileEntry>, AppError> {
        let contents = self.ensure_loaded()?;
        match self.view.get() {
            TorrentView::All => {
                let mut rows = list_level(&contents.entries, &path);
                let dir = normalize_internal_path(&path);
                let stats = crate::torrent_session::stats(&self.relative_path_in_parent);
                if let Some(stats) = &stats {
                    self.remember_what_is_downloaded(&contents, stats);
                }
                let remembered: Vec<usize> =
                    crate::torrent_session::read_state(&self.relative_path_in_parent)
                        .map(|state| state.done)
                        .unwrap_or_default();
                let held = crate::torrent_session::read_state(&self.relative_path_in_parent);
                let chosen = crate::selection::spelled_out(&contents.entries, held.as_ref());
                for row in rows.iter_mut() {
                    let stands_for =
                        crate::selection::ids_for_row(&contents.entries, &dir, &row.name);
                    let ticked = if crate::selection::is_ticked(&chosen, &stands_for) {
                        crate::rows::TICKED.to_string()
                    } else {
                        String::new()
                    };
                    let status = match (&stats, row.is_dir) {
                        _ if !crate::selection::is_ticked(&chosen, &stands_for) => String::new(),
                        (Some(stats), false) => {
                            let full = if dir.is_empty() {
                                row.name.clone()
                            } else {
                                format!("{}/{}", dir, row.name)
                            };
                            match contents.entries.iter().find(|e| e.path == full) {
                                Some(e) => {
                                    let done = stats.file_progress.get(e.id).copied().unwrap_or(0);
                                    status_text(done, e.size)
                                }
                                None => String::new(),
                            }
                        }
                        (None, false) if stands_for.iter().all(|id| remembered.contains(id)) => {
                            crate::i18n::tr("torrent.status_ready")
                        }
                        _ => String::new(),
                    };
                    row.extra = vec![ticked, status];
                }
                Ok(rows)
            }
            TorrentView::Downloaded => {
                let key = self.relative_path_in_parent.clone();
                Ok(downloaded_rows(&contents.entries, &|id, size| {
                    crate::torrent_session::is_file_complete(&key, id, size)
                }))
            }
            TorrentView::Peers => Ok(peer_rows(&crate::torrent_session::peers(
                &self.relative_path_in_parent,
            ))),
        }
    }

    pub fn read_file(&self, path: String) -> Result<Vec<u8>, AppError> {
        let wanted = normalize_internal_path(&path);
        if self.view.get() == TorrentView::Peers {
            let peers = crate::torrent_session::peers(&self.relative_path_in_parent);
            return peers
                .iter()
                .find(|p| p.addr == wanted)
                .map(|p| crate::torrent_session::peer_card(p).into_bytes())
                .ok_or_else(|| AppError::Other("this peer is no longer connected".to_string()));
        }
        let contents = self.ensure_loaded()?;
        let entry = contents
            .entries
            .iter()
            .find(|e| e.path == wanted)
            .ok_or_else(|| AppError::Other("no such file in this torrent".to_string()))?;
        if !crate::torrent_session::is_file_complete(
            &self.relative_path_in_parent,
            entry.id,
            entry.size,
        ) {
            return Err(AppError::Other(
                "this file has not been downloaded yet".to_string(),
            ));
        }
        let on_disk = crate::torrent_session::downloaded_file_path(
            &self.relative_path_in_parent,
            &entry.path,
        );
        std::fs::read(on_disk).map_err(AppError::from)
    }

    pub fn extra_columns(&self) -> Vec<ColumnSpec> {
        match self.view.get() {
            TorrentView::All => vec![
                // First, so the box reads as part of the row.
                ColumnSpec {
                    key: FETCH_COLUMN.to_string(),
                    title: crate::i18n::tr("torrent.col_fetch"),
                    width: Some(60),
                    kind: crate::rows::ColumnKind::Check,
                },
                ColumnSpec {
                    key: "torrent_status".to_string(),
                    title: crate::i18n::tr("torrent.col_status"),
                    width: Some(110),
                    kind: crate::rows::ColumnKind::Text,
                },
            ],
            TorrentView::Downloaded => Vec::new(),
            TorrentView::Peers => vec![
                ColumnSpec {
                    key: "peer_state".to_string(),
                    title: crate::i18n::tr("torrent.col_state"),
                    width: Some(110),
                    kind: crate::rows::ColumnKind::Text,
                },
                ColumnSpec {
                    key: "peer_pieces".to_string(),
                    title: crate::i18n::tr("torrent.col_pieces"),
                    width: Some(90),
                    kind: crate::rows::ColumnKind::Text,
                },
                ColumnSpec {
                    key: "peer_errors".to_string(),
                    title: crate::i18n::tr("torrent.col_errors"),
                    width: Some(90),
                    kind: crate::rows::ColumnKind::Text,
                },
            ],
        }
    }

    /// False so a downloaded file can be deleted; every other write is refused per call.
    pub fn is_read_only(&self) -> bool {
        false
    }

    pub fn cannot_be_written_to() -> AppError {
        AppError::Other("a torrent cannot be written to, only downloaded".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(paths: &[(&str, u64)]) -> Vec<TorrentEntry> {
        paths
            .iter()
            .enumerate()
            .map(|(id, (p, s))| TorrentEntry {
                id,
                path: (*p).to_string(),
                size: *s,
            })
            .collect()
    }

    #[test]
    fn a_single_file_torrent_lists_that_file_at_the_root() {
        let e = entries(&[("ubuntu.iso", 4096)]);
        let listing = list_level(&e, "");
        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0].name, "ubuntu.iso");
        assert!(!listing[0].is_dir);
        assert_eq!(listing[0].size, 4096);
    }

    #[test]
    fn nested_paths_become_directories_at_the_level_above() {
        let e = entries(&[
            ("disc/track1.flac", 10),
            ("disc/track2.flac", 20),
            ("readme.txt", 5),
        ]);
        let listing = list_level(&e, "");
        let names: Vec<&str> = listing.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["disc", "readme.txt"]);
        assert!(listing[0].is_dir);
        assert_eq!(listing[0].size, 0);
    }

    #[test]
    fn descending_into_a_directory_lists_its_own_children() {
        let e = entries(&[("disc/track1.flac", 10), ("disc/art/cover.jpg", 7)]);
        let listing = list_level(&e, "disc");
        let names: Vec<&str> = listing.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["art", "track1.flac"]);
        assert!(listing[0].is_dir);
        assert!(!listing[1].is_dir);
        assert_eq!(listing[1].size, 10);
    }

    #[test]
    fn a_deeper_level_is_reached_through_its_full_prefix() {
        let e = entries(&[("disc/art/cover.jpg", 7), ("disc/art/back.jpg", 9)]);
        let listing = list_level(&e, "disc/art");
        let names: Vec<&str> = listing.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["back.jpg", "cover.jpg"]);
    }

    #[test]
    fn a_sibling_directory_does_not_leak_into_the_listing() {
        let e = entries(&[("a/one.bin", 1), ("ab/two.bin", 2)]);
        let listing = list_level(&e, "a");
        let names: Vec<&str> = listing.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["one.bin"]);
    }

    #[test]
    fn a_path_is_normalized_before_it_is_matched() {
        let e = entries(&[("disc/track1.flac", 10)]);
        assert_eq!(list_level(&e, "/disc/").len(), 1);
        assert_eq!(list_level(&e, "\\disc").len(), 1);
    }

    #[test]
    fn an_unknown_directory_lists_nothing() {
        let e = entries(&[("disc/track1.flac", 10)]);
        assert!(list_level(&e, "missing").is_empty());
    }

    #[test]
    fn garbage_is_not_mistaken_for_a_torrent() {
        assert!(parse_torrent(b"not a torrent at all").is_err());
        assert!(parse_torrent(b"").is_err());
    }
    fn single_file_torrent_bytes() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"d8:announce3:abc4:infod6:lengthi1024e4:name8:test.bin12:piece lengthi16384e6:pieces20:");
        v.extend_from_slice(&[0u8; 20]);
        v.extend_from_slice(b"ee");
        v
    }

    fn multi_file_torrent_bytes() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"d8:announce3:abc4:infod5:filesld6:lengthi10e4:pathl4:disc11:track1.flaceed6:lengthi20e4:pathl9:cover.jpgeee4:name5:album12:piece lengthi16384e6:pieces20:");
        v.extend_from_slice(&[0u8; 20]);
        v.extend_from_slice(b"ee");
        v
    }

    #[test]
    fn a_real_single_file_torrent_parses_to_one_entry() {
        let c = parse_torrent(&single_file_torrent_bytes()).unwrap();
        assert_eq!(c.name, "test.bin");
        assert_eq!(
            c.entries,
            vec![TorrentEntry {
                id: 0,
                path: "test.bin".to_string(),
                size: 1024
            }]
        );
    }

    #[test]
    fn a_real_multi_file_torrent_keeps_its_directory_structure() {
        let c = parse_torrent(&multi_file_torrent_bytes()).unwrap();
        assert_eq!(c.name, "album");
        assert_eq!(c.entries.len(), 2);
        assert_eq!(c.entries[0].path, "disc/track1.flac");
        assert_eq!(c.entries[0].size, 10);
        assert_eq!(c.entries[1].path, "cover.jpg");
    }

    #[test]
    fn a_parsed_multi_file_torrent_lists_like_a_directory_tree() {
        let c = parse_torrent(&multi_file_torrent_bytes()).unwrap();
        let root: Vec<String> = list_level(&c.entries, "")
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(root, vec!["cover.jpg".to_string(), "disc".to_string()]);
        let inner = list_level(&c.entries, "disc");
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].name, "track1.flac");
        assert_eq!(inner[0].size, 10);
    }

    #[test]
    #[ignore]
    fn a_torrent_file_from_disk_parses() {
        let path = match std::env::var("IC_TORRENT_FILE") {
            Ok(p) => p,
            Err(_) => return,
        };
        let bytes = std::fs::read(&path).expect("torrent file is readable");
        let c = parse_torrent(&bytes).expect("torrent file parses");
        assert!(!c.entries.is_empty());
        assert!(!list_level(&c.entries, "").is_empty());
    }

    fn torrent_with_a_padding_file_bytes() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"d8:announce3:abc4:infod5:filesl");
        v.extend_from_slice(b"d6:lengthi10e4:pathl4:disc11:track1.flacee");
        v.extend_from_slice(b"d4:attr1:p6:lengthi5e4:pathl6:.pad_5ee");
        v.extend_from_slice(b"d6:lengthi20e4:pathl9:cover.jpgee");
        v.extend_from_slice(b"e4:name5:album12:piece lengthi16384e6:pieces20:");
        v.extend_from_slice(&[0u8; 20]);
        v.extend_from_slice(b"ee");
        v
    }

    #[test]
    fn a_padding_file_is_hidden_but_still_consumes_its_index() {
        let c = parse_torrent(&torrent_with_a_padding_file_bytes()).unwrap();
        assert_eq!(c.entries.len(), 2);
        assert_eq!(c.entries[0].path, "disc/track1.flac");
        assert_eq!(c.entries[0].id, 0);
        assert_eq!(c.entries[1].path, "cover.jpg");
        assert_eq!(
            c.entries[1].id, 2,
            "the padding file at index 1 must keep its slot, or only_files would select the wrong file"
        );
    }

    #[test]
    fn a_file_with_nothing_downloaded_shows_no_status() {
        assert_eq!(status_text(0, 100), "");
    }

    #[test]
    fn a_partly_downloaded_file_shows_a_percentage() {
        assert_eq!(status_text(50, 100), "50%");
        assert_eq!(status_text(1, 100), "1%");
    }

    #[test]
    fn a_percentage_never_reads_as_a_hundred_before_it_is_finished() {
        assert_eq!(status_text(999, 1000), "99%");
    }

    #[test]
    fn an_empty_file_never_looks_partly_downloaded() {
        assert_eq!(status_text(0, 0), "");
    }
}
