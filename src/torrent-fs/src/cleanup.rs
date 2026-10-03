use ic_plugin_api::{IcBytes, IcViewVTable};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::os::raw::c_void;

pub const VIEW_ID: &str = "torrent.cleanup";

// One dialogue at a time: the host reuses the open view for a second `open_view` of the same id.
thread_local! {
    static ASKING_ABOUT: RefCell<String> = const { RefCell::new(String::new()) };
    static ANSWER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

pub fn about(path: &str) -> String {
    ASKING_ABOUT.with(|held| *held.borrow_mut() = path.to_string());
    json!({ "path": path }).to_string()
}

fn phrase(key: &str, english: &str) -> Value {
    json!({ "tr": key, "en": english })
}

pub fn document() -> Value {
    json!({
        "schema": 1,
        "kind": VIEW_ID,
        "fields": [
            { "bind": "also_torrent", "type": "bool", "default": false }
        ],
        "form": {
            "t": "view",
            "id": "cleanup",
            "padding": 20,
            "width": 460,
            "title": phrase("torrent.cleanup_heading", "Remove downloaded data?"),
            "children": [
                { "t": "text", "id": "body", "wrap": true,
                  "text": phrase("torrent.cleanup_body",
                    "This deletes the folder with the partial data and the state file next to the torrent.") },
                { "t": "text", "id": "which", "role": "dim", "margin_top": 8,
                  "text": "{arg.path}" },
                { "t": "switch", "id": "also_torrent", "bind": "also_torrent",
                  "margin_top": 12,
                  "title": phrase("torrent.cleanup_delete_file",
                    "Delete the .torrent file as well") },
                { "t": "row", "id": "buttons", "spacing": 8, "margin_top": 20, "children": [
                    { "t": "button", "id": "cancel", "weight": 1,
                      "label": phrase("common.cancel", "Cancel"),
                      "intent": { "do": "close" } },
                    { "t": "button", "id": "wipe", "role": "destructive",
                      "label": phrase("torrent.cleanup_confirm", "Clean up"),
                      "intent": { "do": "emit", "node": "wipe" } } ] }
            ]
        }
    })
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
    answer_with(&document().to_string())
}

/// The path comes from what was stored on open, never from the user-editable form.
fn reply_for(event: &Value) -> Value {
    if event["node"].as_str() != Some("wipe") {
        return json!({});
    }
    let path = ASKING_ABOUT.with(|held| held.borrow().clone());
    if path.is_empty() {
        return json!({ "close": true });
    }
    let also_torrent = event["values"]["also_torrent"].as_bool().unwrap_or(false);
    crate::in_the_background(move || async move {
        crate::torrent_session::cleanup(&path, also_torrent).await
    });
    json!({ "close": true })
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

    #[test]
    fn the_dialogue_offers_the_way_out_as_well_as_the_way_through() {
        let doc = document();
        let buttons = doc["form"]["children"]
            .as_array()
            .expect("children")
            .iter()
            .find(|node| node["id"] == "buttons")
            .expect("a row of buttons");
        let ids: Vec<&str> = buttons["children"]
            .as_array()
            .expect("buttons")
            .iter()
            .map(|node| node["id"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(ids, vec!["cancel", "wipe"]);
        assert_eq!(buttons["children"][0]["intent"]["do"], "close");
        assert_eq!(
            buttons["children"][1]["role"], "destructive",
            "deleting should not look like the ordinary way on"
        );
    }

    #[test]
    fn the_checkbox_starts_unticked_so_the_torrent_is_kept_by_default() {
        let doc = document();
        assert_eq!(doc["fields"][0]["bind"], "also_torrent");
        assert_eq!(doc["fields"][0]["default"], json!(false));
    }

    #[test]
    fn nothing_happens_until_the_destructive_button_is_the_one_pressed() {
        assert_eq!(reply_for(&json!({ "node": "cancel" })), json!({}));
        assert_eq!(reply_for(&json!({ "node": "also_torrent" })), json!({}));
        assert_eq!(reply_for(&Value::Null), json!({}));
    }

    #[test]
    fn a_confirmation_about_no_torrent_at_all_just_shuts_the_dialogue() {
        ASKING_ABOUT.with(|held| held.borrow_mut().clear());
        assert_eq!(
            reply_for(&json!({ "node": "wipe", "values": {} })),
            json!({ "close": true })
        );
    }

    #[test]
    fn opening_the_dialogue_says_which_torrent_it_is_about() {
        let carried = about("/tmp/holiday.torrent");
        assert_eq!(
            serde_json::from_str::<Value>(&carried).expect("json")["path"],
            "/tmp/holiday.torrent"
        );
        assert_eq!(
            ASKING_ABOUT.with(|held| held.borrow().clone()),
            "/tmp/holiday.torrent"
        );
    }
}
