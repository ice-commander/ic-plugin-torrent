use crate::torrent_rpc::TorrentEntry;

pub fn inside(torrent_path: &str, picked: &str) -> Option<String> {
    let tidy = |text: &str| text.replace('\\', "/").trim_end_matches('/').to_string();
    let torrent = tidy(torrent_path);
    let picked = tidy(picked);
    let rest = picked.strip_prefix(&torrent)?;
    let rest = rest.trim_start_matches('/');
    (!rest.is_empty()).then(|| rest.to_string())
}

/// Torrent indices, not listing positions: a hidden padding file keeps its slot.
pub fn chosen(entries: &[TorrentEntry], torrent_path: &str, picked: &[String]) -> Vec<usize> {
    let wanted: Vec<String> = picked
        .iter()
        .filter_map(|one| inside(torrent_path, one))
        .collect();
    let mut found: Vec<usize> = entries
        .iter()
        .filter(|entry| {
            wanted
                .iter()
                .any(|one| entry.path == *one || entry.path.starts_with(&format!("{one}/")))
        })
        .map(|entry| entry.id)
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

pub fn ids_for_row(entries: &[TorrentEntry], dir: &str, name: &str) -> Vec<usize> {
    let dir = dir.replace('\\', "/");
    let dir = dir.trim_matches('/');
    let full = if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    };
    entries
        .iter()
        .filter(|entry| entry.path == full || entry.path.starts_with(&format!("{full}/")))
        .map(|entry| entry.id)
        .collect()
}

pub fn spelled_out(
    _entries: &[TorrentEntry],
    held: Option<&crate::torrent_session::TorrentState>,
) -> Vec<usize> {
    held.map(|state| state.selected.clone()).unwrap_or_default()
}

pub fn is_ticked(chosen: &[usize], row: &[usize]) -> bool {
    !row.is_empty() && row.iter().all(|id| chosen.contains(id))
}

pub fn after_click(chosen: &[usize], row: &[usize], ticked: bool) -> Vec<usize> {
    let mut next: Vec<usize> = chosen.to_vec();
    if ticked {
        for id in row {
            if !next.contains(id) {
                next.push(*id);
            }
        }
    } else {
        next.retain(|id| !row.contains(id));
    }
    next.sort_unstable();
    next.dedup();
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<TorrentEntry> {
        [
            ("disc/one.flac", 10u64),
            ("disc/two.flac", 20),
            ("disc/inner/three.flac", 30),
            ("cover.jpg", 40),
        ]
        .iter()
        .enumerate()
        .map(|(id, (path, size))| TorrentEntry {
            id,
            path: (*path).to_string(),
            size: *size,
        })
        .collect()
    }

    const TORRENT: &str = "/home/u/holiday.torrent";

    #[test]
    fn a_row_stands_for_the_file_it_names() {
        assert_eq!(ids_for_row(&entries(), "", "cover.jpg"), vec![3]);
        assert_eq!(ids_for_row(&entries(), "disc", "one.flac"), vec![0]);
        assert_eq!(ids_for_row(&entries(), "/disc/", "two.flac"), vec![1]);
    }

    #[test]
    fn a_row_that_is_a_directory_stands_for_everything_under_it() {
        assert_eq!(ids_for_row(&entries(), "", "disc"), vec![0, 1, 2]);
        assert_eq!(ids_for_row(&entries(), "disc", "inner"), vec![2]);
    }

    #[test]
    fn a_row_never_reaches_into_a_directory_that_merely_starts_the_same() {
        let entries = vec![
            TorrentEntry {
                id: 0,
                path: "disc/one.flac".to_string(),
                size: 1,
            },
            TorrentEntry {
                id: 1,
                path: "discography/two.flac".to_string(),
                size: 1,
            },
        ];
        assert_eq!(ids_for_row(&entries, "", "disc"), vec![0]);
    }

    #[test]
    fn a_row_naming_nothing_in_the_torrent_stands_for_nothing() {
        assert!(ids_for_row(&entries(), "", "elsewhere.txt").is_empty());
        assert!(ids_for_row(&entries(), "discography", "one.flac").is_empty());
    }

    fn state(selected: Vec<usize>) -> crate::torrent_session::TorrentState {
        crate::torrent_session::TorrentState {
            info_hash: "abc".to_string(),
            selected,
            done: Vec::new(),
        }
    }

    #[test]
    fn a_torrent_nobody_has_chosen_from_has_chosen_nothing() {
        assert!(
            spelled_out(&entries(), None).is_empty(),
            "a file downloaded and never opened should fetch nothing"
        );
        assert!(spelled_out(&entries(), Some(&state(Vec::new()))).is_empty());
    }

    #[test]
    fn a_choice_that_was_written_down_is_taken_as_it_is() {
        assert_eq!(
            spelled_out(&entries(), Some(&state(vec![1, 3]))),
            vec![1, 3]
        );
    }

    #[test]
    fn a_box_is_ticked_when_every_file_behind_it_is_being_fetched() {
        assert!(is_ticked(&[0, 1, 2], &[0]));
        assert!(is_ticked(&[0, 1, 2], &[0, 1, 2]));
        assert!(!is_ticked(&[0, 1], &[0, 1, 2]));
        assert!(!is_ticked(&[], &[0]));
    }

    #[test]
    fn a_row_standing_for_nothing_is_not_ticked() {
        assert!(!is_ticked(&[0, 1, 2], &[]));
    }

    #[test]
    fn ticking_a_box_adds_what_it_stands_for() {
        assert_eq!(after_click(&[0], &[2], true), vec![0, 2]);
        assert_eq!(after_click(&[0], &[0, 1, 2], true), vec![0, 1, 2]);
    }

    #[test]
    fn unticking_a_box_takes_away_what_it_stands_for_and_nothing_else() {
        assert_eq!(after_click(&[0, 1, 2, 3], &[1], false), vec![0, 2, 3]);
        assert_eq!(after_click(&[0, 1, 2, 3], &[0, 1, 2], false), vec![3]);
    }

    #[test]
    fn clicking_the_same_way_twice_changes_nothing_the_second_time() {
        let once = after_click(&[0], &[2], true);
        assert_eq!(after_click(&once, &[2], true), once);
        let off = after_click(&[0, 2], &[2], false);
        assert_eq!(after_click(&off, &[2], false), off);
    }

    #[test]
    fn unticking_the_last_file_leaves_an_empty_choice_not_a_full_one() {
        assert!(after_click(&[1], &[1], false).is_empty());
    }

    #[test]
    fn a_path_the_panel_shows_is_cut_down_to_what_is_inside_the_torrent() {
        assert_eq!(
            inside(TORRENT, "/home/u/holiday.torrent/disc/one.flac").as_deref(),
            Some("disc/one.flac")
        );
        assert_eq!(
            inside(TORRENT, "/home/u/holiday.torrent/cover.jpg").as_deref(),
            Some("cover.jpg")
        );
    }

    #[test]
    fn the_torrent_itself_names_no_file_inside_it() {
        assert_eq!(inside(TORRENT, TORRENT), None);
        assert_eq!(inside(TORRENT, "/home/u/holiday.torrent/"), None);
    }

    #[test]
    fn something_from_another_panel_is_not_mistaken_for_ours() {
        assert_eq!(inside(TORRENT, "/home/u/other.torrent/disc/one.flac"), None);
        assert_eq!(inside(TORRENT, "/etc/passwd"), None);
    }

    #[test]
    fn windows_separators_are_read_the_same_way() {
        assert_eq!(
            inside(
                "C:/u/holiday.torrent",
                "C:\\u\\holiday.torrent\\disc\\one.flac"
            )
            .as_deref(),
            Some("disc/one.flac")
        );
    }

    #[test]
    fn picking_a_file_chooses_that_one_file() {
        assert_eq!(
            chosen(
                &entries(),
                TORRENT,
                &["/home/u/holiday.torrent/cover.jpg".to_string()]
            ),
            vec![3]
        );
    }

    #[test]
    fn picking_a_directory_brings_everything_under_it() {
        assert_eq!(
            chosen(
                &entries(),
                TORRENT,
                &["/home/u/holiday.torrent/disc".to_string()]
            ),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn a_directory_never_reaches_past_its_own_name() {
        let entries = vec![
            TorrentEntry {
                id: 0,
                path: "disc/one.flac".to_string(),
                size: 1,
            },
            TorrentEntry {
                id: 1,
                path: "discography/two.flac".to_string(),
                size: 1,
            },
        ];
        assert_eq!(
            chosen(
                &entries,
                TORRENT,
                &["/home/u/holiday.torrent/disc".to_string()]
            ),
            vec![0]
        );
    }

    #[test]
    fn a_file_picked_twice_over_is_counted_once() {
        assert_eq!(
            chosen(
                &entries(),
                TORRENT,
                &[
                    "/home/u/holiday.torrent/disc".to_string(),
                    "/home/u/holiday.torrent/disc/one.flac".to_string(),
                ]
            ),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn picking_nothing_chooses_nothing() {
        assert!(chosen(&entries(), TORRENT, &[]).is_empty());
        assert!(chosen(&entries(), TORRENT, &["/elsewhere/x".to_string()]).is_empty());
    }

    #[test]
    fn the_indices_are_the_torrents_own_not_the_listings() {
        let entries = vec![
            TorrentEntry {
                id: 0,
                path: "one.flac".to_string(),
                size: 1,
            },
            TorrentEntry {
                id: 2,
                path: "two.flac".to_string(),
                size: 1,
            },
        ];
        assert_eq!(
            chosen(
                &entries,
                TORRENT,
                &["/home/u/holiday.torrent/two.flac".to_string()]
            ),
            vec![2]
        );
    }
}
