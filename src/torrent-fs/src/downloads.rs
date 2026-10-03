use ic_plugin_api::{IcBytes, IcViewVTable};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::os::raw::c_void;

pub const BUTTON_ID: &str = "torrent.activity";
pub const VIEW_ID: &str = "torrent.downloads";
pub const ICON: &str = include_str!("../assets/torrent.svg");

thread_local! {
    static ANSWER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

pub fn speeds(activity: &crate::torrent_session::Activity) -> String {
    format!(
        "\u{2193} {}  \u{2191} {}",
        crate::torrent_session::format_speed(activity.download_mbps),
        crate::torrent_session::format_speed(activity.upload_mbps)
    )
}

pub fn short_name(path: &str) -> String {
    path.replace('\\', "/")
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn phrase(key: &str, english: &str) -> Value {
    json!({ "tr": key, "en": english })
}

pub fn document(running: &[crate::torrent_session::Running]) -> Value {
    let mut children: Vec<Value> = vec![json!({
        "t": "text",
        "id": "heading",
        "text": phrase("torrent.downloads_heading", "Downloads")
    })];

    if running.is_empty() {
        children.push(json!({
            "t": "text",
            "id": "nothing",
            "role": "dim",
            "margin_top": 8,
            "text": phrase("torrent.downloads_none", "Nothing is downloading.")
        }));
    }

    for (at, one) in running.iter().enumerate() {
        children.push(json!({
            "t": "row",
            "id": format!("row{at}"),
            "spacing": 12,
            "margin_top": 12,
            "children": [
                { "t": "column", "id": format!("what{at}"), "weight": 1, "children": [
                    { "t": "text", "id": format!("name{at}"),
                      "text": { "literal": short_name(&one.path) } },
                    { "t": "text", "id": format!("how{at}"), "role": "dim",
                      "text": { "literal": format!(
                          "{}%  \u{2193} {}",
                          one.percent(),
                          crate::torrent_session::format_speed(one.download_mbps)
                      ) } }
                ] },
                { "t": "button", "id": format!("stop{at}"),
                  "label": phrase("torrent.stop", "Stop"),
                  "intent": { "do": "emit", "node": format!("stop{at}") } }
            ]
        }));
    }

    children.push(json!({
        "t": "row",
        "id": "buttons",
        "spacing": 8,
        "margin_top": 20,
        "children": [
            { "t": "text", "id": "spacer", "weight": 1, "text": { "literal": "" } },
            { "t": "button", "id": "close",
              "label": phrase("torrent.close", "Close"),
              "intent": { "do": "close" } }
        ]
    }));

    json!({
        "schema": 1,
        "kind": VIEW_ID,
        "fields": [],
        "form": {
            "t": "view",
            "id": "downloads",
            "padding": 20,
            "width": 460,
            "title": phrase("torrent.downloads_heading", "Downloads"),
            "children": children
        }
    })
}

pub fn stopping(node: &str, running: &[crate::torrent_session::Running]) -> Option<String> {
    let at: usize = node.strip_prefix("stop")?.parse().ok()?;
    running.get(at).map(|one| one.path.clone())
}

fn answer_with(source: &str) -> IcBytes {
    ANSWER.with(|slot| {
        *slot.borrow_mut() = source.as_bytes().to_vec();
        let held = slot.borrow();
        IcBytes {
            data: held.as_ptr(),
            len: held.len() as u64,
        }
    })
}

extern "C" fn describe(_ctx: *const u8, _ctx_len: u64, _user_data: *mut c_void) -> IcBytes {
    answer_with(&document(&crate::torrent_session::running()).to_string())
}

fn reply_for(event: &Value) -> Value {
    let Some(node) = event["node"].as_str() else {
        return json!({});
    };
    let running = crate::torrent_session::running();
    let Some(path) = stopping(node, &running) else {
        return json!({});
    };
    crate::in_the_background(move || async move { crate::torrent_session::stop(&path).await });
    // Drawn again now and once the stop lands: pausing takes a moment.
    json!({ "redescribe": true })
}

extern "C" fn on_event(event: *const u8, len: u64, _user_data: *mut c_void) -> IcBytes {
    if event.is_null() || len == 0 {
        return answer_with("{}");
    }
    let raw = unsafe { std::slice::from_raw_parts(event, len as usize) };
    let parsed: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    answer_with(&reply_for(&parsed).to_string())
}

pub fn vtable() -> IcViewVTable {
    IcViewVTable {
        struct_size: std::mem::size_of::<IcViewVTable>() as u32,
        describe,
        on_event: Some(on_event),
        closed: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::torrent_session::Running;

    fn one(path: &str, done: u64, total: u64) -> Running {
        Running {
            path: path.to_string(),
            progress_bytes: done,
            total_bytes: total,
            download_mbps: 1.5,
        }
    }

    #[test]
    fn every_torrent_gets_a_row_and_a_way_to_stop_it() {
        let doc = document(&[one("/home/u/a.torrent", 1, 2), one("/x/b.torrent", 1, 4)]);
        let children = doc["form"]["children"].as_array().expect("children");
        let stops: Vec<&str> = children
            .iter()
            .filter(|node| {
                node["id"]
                    .as_str()
                    .map(|id| id.starts_with("row"))
                    .unwrap_or(false)
            })
            .filter_map(|row| row["children"].as_array())
            .filter_map(|row| row.last())
            .filter_map(|button| button["id"].as_str())
            .collect();
        assert_eq!(stops, vec!["stop0", "stop1"]);
        let text = doc.to_string();
        assert!(text.contains("a.torrent") && text.contains("b.torrent"));
    }

    #[test]
    fn an_empty_list_says_so_rather_than_showing_nothing_at_all() {
        let doc = document(&[]);
        let ids: Vec<&str> = doc["form"]["children"]
            .as_array()
            .expect("children")
            .iter()
            .filter_map(|node| node["id"].as_str())
            .collect();
        assert!(ids.contains(&"nothing"));
    }

    #[test]
    fn a_stop_button_names_the_torrent_that_was_in_that_place() {
        let running = vec![one("/home/u/a.torrent", 1, 2), one("/x/b.torrent", 1, 4)];
        assert_eq!(
            stopping("stop0", &running).as_deref(),
            Some("/home/u/a.torrent")
        );
        assert_eq!(stopping("stop1", &running).as_deref(), Some("/x/b.torrent"));
    }

    #[test]
    fn a_press_that_lands_after_the_torrent_has_gone_stops_nothing() {
        let running = vec![one("/home/u/a.torrent", 1, 2)];
        assert_eq!(stopping("stop1", &running), None);
        assert_eq!(stopping("close", &running), None);
        assert_eq!(stopping("stopnowhere", &running), None);
        assert_eq!(stopping("", &running), None);
    }

    #[test]
    fn a_torrent_is_shown_by_its_file_rather_than_its_whole_path() {
        assert_eq!(
            short_name("/home/u/Holiday Photos.torrent"),
            "Holiday Photos.torrent"
        );
        assert_eq!(short_name("a.torrent"), "a.torrent");
        assert_eq!(short_name("C:\\\\users\\\\u\\\\a.torrent"), "a.torrent");
        assert_eq!(short_name("/"), "/");
    }

    #[test]
    fn the_header_says_both_speeds() {
        let said = speeds(&crate::torrent_session::Activity {
            torrents: 2,
            download_mbps: 1.5,
            upload_mbps: 0.0,
        });
        assert_eq!(said, "\u{2193} 1.5 MB/s  \u{2191} 0");
    }
}
